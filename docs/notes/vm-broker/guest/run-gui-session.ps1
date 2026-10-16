# Run the ignored winit window tests INSIDE the guest's interactive session.
#
# Allow-listed guest script. Called by the broker exactly like run-tests.ps1:
#
#   broker -Op guest -Script run-gui-session.ps1 -GuestArgs <filter>
#
# Why this script exists: the broker's PowerShell Direct sessions land in
# session 0 as the S4U user, which has no desktop. The plugin host's plain
# Win32 window and the end-to-end test pass there, but every winit window test
# panics inside winit without a desktop. The guest DOES have an auto-logon
# interactive session (see docs\notes\vm-provision-test-host.ps1), so this
# script registers a one-shot Scheduled Task that runs as the interactive user
# with LogonType Interactive - i.e. in that session, on its desktop - and waits
# for it. The task-registration shape follows the precedent of
# docs\notes\vm-broker\register-broker-task.ps1 (action + principal + trigger),
# moved inside the guest and pointed at the console session instead of S4U.
#
# Division of labour (unchanged): the host compiled and staged unelevated, the
# broker only touches the VM, this script only runs what is staged. The
# executed payload is a fixed local path written by THIS allow-listed script,
# never an argument from the broker, so the allow-list model holds: the only
# broker-controlled input is the optional test-name filter.
#
# Trust: the run reports its session id, user, computer and the window client
# sizes/positions the tests assert. A run that lands in session 0 FAILS LOUDLY
# (three layers: the Interactive principal cannot start without the logon, the
# inner script refuses session 0 before any binary runs, the outer wrapper
# refuses a proof file showing session 0) instead of reporting a pass.
#
# Results land in C:\rustamp\out\gui-session\ and come back via the existing
# broker `copyout` op (source under C:\rustamp\out, as that op requires).

param([string]$Filter = '')

$ErrorActionPreference = 'Stop'

$TaskName = 'RustampGuiSession'
$WorkDir  = 'C:\rustamp\out\gui-session'
$InnerPath = Join-Path $WorkDir 'inner.ps1'
$DonePath  = Join-Path $WorkDir 'done.json'
$ProofPath = Join-Path $WorkDir 'session-proof.json'

function Session-Of([int]$ProcessId) {
    (Get-Process -Id $ProcessId -ErrorAction Stop).SessionId
}

# --- 0. record where THIS script landed (must be session 0; informational) ----
$launcherSession = Session-Of $PID
Write-Output ("launcher session {0} as {1} on {2}" -f $launcherSession, $env:USERNAME, $env:COMPUTERNAME)

# --- 1. make the baked host paths resolve (same trick as run-tests.ps1) -------
if (-not (Test-Path 'D:\code\Rustamp')) {
    subst D: C:\rustamp
}
$repo = 'D:\code\Rustamp'
if (-not (Test-Path $repo)) { throw "the repository is not staged at $repo" }
$deps = Join-Path $repo 'target\debug\deps'
if (-not (Test-Path $deps)) { throw "no staged binaries at $deps" }

# --- 2. find the interactive session and its user ------------------------------
# Primary signal: explorer.exe owns the console desktop; its session is the
# interactive one and its owner is the auto-logon user. Fallback: `query
# session` looking for the Active console row. No hardcoded user name: the
# provision script sets auto-logon to whoever provisioned, and either signal
# follows it.
$interactiveSession = $null
$interactiveUser = $null
$interactiveDomain = $null
$explorer = Get-Process -Name explorer -ErrorAction SilentlyContinue | Select-Object -First 1
if ($explorer) {
    $interactiveSession = $explorer.SessionId
    $owner = (Get-CimInstance Win32_Process -Filter ("ProcessId = " + $explorer.Id) | Invoke-CimMethod -MethodName GetOwner)
    $interactiveUser = $owner.User
    $interactiveDomain = $owner.Domain
    Write-Output ("interactive session {0} via explorer (user {1}\{2})" -f $interactiveSession, $interactiveDomain, $interactiveUser)
} else {
    Write-Output 'no explorer process; falling back to `query session`'
    foreach ($line in (query session 2>$null)) {
        # Active console row looks like: ">console  rustamp  1  Active"
        if ($line -match '^\s*>?\s*console\s+(\S+)\s+(\d+)\s+Active') {
            $interactiveUser = $Matches[1]
            $interactiveSession = [int]$Matches[2]
            $interactiveDomain = $env:COMPUTERNAME
            Write-Output ("interactive session {0} via query session (user {1})" -f $interactiveSession, $interactiveUser)
            break
        }
    }
}
if ($null -eq $interactiveSession) { throw 'no interactive session found: explorer is not running and `query session` shows no Active console; the window tests have no desktop to run on' }
if ($interactiveSession -eq 0) { throw 'the "interactive" session resolved to session 0, which has no desktop; refusing instead of reporting a pass' }
if ([string]::IsNullOrWhiteSpace($interactiveUser)) { throw 'interactive session found but its user could not be resolved; refusing' }

# --- 3. write the inner payload (fixed local path, filter baked in) ------------
# The four ignored winit window tests and nothing else. Each runs in its own
# staged binary with `--ignored <exact name>` so a binary-name filter cannot
# accidentally widen the set. The spawn-based cases (playlist_window, headless)
# resolve rustamp.exe through the compile-time path, which works because of the
# subst above - same as the existing guest runs.
$windowCases = @(
    @{ stem = 'eq_window';       test = 'the_equalizer_window_opens_at_its_docked_size_and_position' },
    @{ stem = 'main_menu';       test = 'the_main_menu_popup_opens_at_the_measured_client_size' },
    @{ stem = 'playlist_window'; test = 'the_playlist_window_opens_docked_below_the_main_window' },
    @{ stem = 'headless';        test = 'the_windowed_player_opens_a_window_inside_the_test_vm' }
)
if (-not [string]::IsNullOrWhiteSpace($Filter)) {
    $want = $Filter.Trim()
    $windowCases = @($windowCases | Where-Object { $_.stem -like "*$want*" -or $_.test -like "*$want*" })
    if (-not $windowCases) { throw "filter '$Filter' matches none of the four window tests" }
}
$caseJson = ($windowCases | ConvertTo-Json -Compress)

if (Test-Path $WorkDir) { Remove-Item -Recurse -Force $WorkDir }
$null = New-Item -ItemType Directory -Force -Path $WorkDir

# The filter travels baked into the file (single-quoted here-string: nothing
# expands at write time). The inner script re-asserts the session because the
# only thing that proves the desktop is the process that actually runs there.
$inner = @'
$ErrorActionPreference = 'Continue'
$WorkDir   = 'C:\rustamp\out\gui-session'
$ProofPath = Join-Path $WorkDir 'session-proof.json'
$DonePath  = Join-Path $WorkDir 'done.json'

# Fail loudly in session 0: no desktop there, and a pass from there would be a lie.
$me = (Get-Process -Id $PID).SessionId
if ($me -eq 0) {
    $msg = 'refusing: the GUI task landed in session 0, which has no desktop; a window test cannot pass here'
    Write-Output $msg
    @{ ok = $false; error = $msg } | ConvertTo-Json | Set-Content -Path $DonePath -Encoding UTF8
    exit 3
}

if (-not (Test-Path 'D:\code\Rustamp')) { subst D: C:\rustamp }
$deps = 'D:\code\Rustamp\target\debug\deps'

Add-Type -AssemblyName System.Windows.Forms
$screen = [System.Windows.Forms.SystemInformation]::VirtualScreen
if ($screen.Width -le 0 -or $screen.Height -le 0) {
    $msg = 'refusing: no usable desktop in this session (virtual screen is empty)'
    Write-Output $msg
    @{ ok = $false; error = $msg } | ConvertTo-Json | Set-Content -Path $DonePath -Encoding UTF8
    exit 3
}

$cases = '__CASES__' | ConvertFrom-Json
$results = @()
foreach ($c in $cases) {
    $exe = Get-ChildItem -Path $deps -Filter ($c.stem + '-*.exe') -ErrorAction SilentlyContinue |
        Sort-Object Name | Select-Object -First 1 -ExpandProperty FullName
    if (-not $exe) {
        $results += [pscustomobject]@{ stem = $c.stem; test = $c.test; exit = -1; summary = 'MISSING BINARY'; log = $null }
        continue
    }
    # One log per case so the host can read exactly what the test said,
    # including the client size/position lines the tests assert.
    $logPath = Join-Path $WorkDir ($c.stem + '.log')
    $raw = & $exe --ignored $c.test --exact 2>&1 | Out-String
    $exit = $LASTEXITCODE
    $raw | Set-Content -Path $logPath -Encoding UTF8
    $lines = $raw -split "`r?`n"
    $summary = ($lines | Where-Object { $_ -match '^test result:' } | Select-Object -Last 1)
    $results += [pscustomobject]@{ stem = $c.stem; test = $c.test; exit = $exit; summary = $summary; log = (Split-Path $logPath -Leaf) }
}

[pscustomobject]@{
    session  = $me
    user     = $env:USERNAME
    computer = $env:COMPUTERNAME
    marker   = (Test-Path 'C:\rustamp\code\Rustamp')
    screen   = $screen.ToString()
    ran      = $results.Count
    failed   = @($results | Where-Object { $_.exit -ne 0 }).Count
    results  = $results
} | ConvertTo-Json -Depth 6 | Set-Content -Path $ProofPath -Encoding UTF8

$proof = Get-Content -Path $ProofPath -Raw | ConvertFrom-Json
@{ ok = ($proof.failed -eq 0); failed = $proof.failed } | ConvertTo-Json | Set-Content -Path $DonePath -Encoding UTF8
exit $proof.failed
'@
$inner = $inner.Replace('__CASES__', $caseJson)
$inner | Set-Content -Path $InnerPath -Encoding UTF8

# --- 4. register the one-shot task in the interactive session -------------------
# Precedent: register-broker-task.ps1 (action + principal + trigger). Differences
# are the point: LogonType Interactive (this user, when logged on: the console
# desktop) instead of S4U (session 0), Limited instead of Highest (the tests
# need no elevation; an elevated interactive task would add a UAC question the
# run cannot answer), and a one-shot trigger (this run only, then unregistered).
Unregister-ScheduledTask -TaskName $TaskName -Confirm:$false -ErrorAction SilentlyContinue
$action = New-ScheduledTaskAction -Execute 'powershell.exe' `
    -Argument ('-NoProfile -NonInteractive -ExecutionPolicy Bypass -File "{0}"' -f $InnerPath)
$principal = New-ScheduledTaskPrincipal -UserId ("{0}\{1}" -f $interactiveDomain, $interactiveUser) `
    -LogonType Interactive -RunLevel Limited
$trigger = New-ScheduledTaskTrigger -Once -At (Get-Date).AddMinutes(1)
Register-ScheduledTask -TaskName $TaskName -Action $action -Principal $principal `
    -Trigger $trigger -Force | Out-Null
Write-Output ("registered '{0}' as {1}\{2} (Interactive, session {3}, one-shot)" -f $TaskName, $interactiveDomain, $interactiveUser, $interactiveSession)

# --- 5. run it now and wait for the sentinel ------------------------------------
Start-ScheduledTask -TaskName $TaskName
$deadline = (Get-Date).AddMinutes(20)
$done = $null
while ((Get-Date) -lt $deadline) {
    Start-Sleep -Seconds 5
    if (Test-Path $DonePath) {
        try { $done = Get-Content -Path $DonePath -Raw | ConvertFrom-Json } catch { continue }
        break
    }
    $state = (Get-ScheduledTask -TaskName $TaskName -ErrorAction SilentlyContinue).State
    if ($state -eq 'Disabled') { break }
}
Unregister-ScheduledTask -TaskName $TaskName -Confirm:$false -ErrorAction SilentlyContinue
if (-not $done) { throw 'the GUI task produced no done sentinel within 20 minutes; its logs (if any) are under C:\rustamp\out\gui-session\' }
if (-not (Test-Path $ProofPath)) { throw 'the GUI task finished without a session proof; refusing instead of reporting a pass' }
$proof = Get-Content -Path $ProofPath -Raw | ConvertFrom-Json

# --- 6. trust checks: session 0 must fail loudly ---------------------------------
# The tests' own require_test_vm gate already refused off-VM; this is the
# session half: prove the desktop session, then report the window facts.
if ($proof.session -eq 0) { throw 'the GUI task ran in session 0, which has no desktop; this run proves nothing and is reported as failed' }
if ($proof.session -ne $interactiveSession) {
    throw ("the GUI task ran in session {0}, not the interactive session {1}; refusing" -f $proof.session, $interactiveSession)
}
Write-Output ("gui session {0} as {1} on {2}; screen {3}; ran {4}, failed {5}" -f `
    $proof.session, $proof.user, $proof.computer, $proof.screen, $proof.ran, $proof.failed)
foreach ($r in $proof.results) {
    $status = if ($r.exit -eq 0) { 'ok  ' } else { 'FAIL' }
    Write-Output ("  [{0}] {1,-16} {2} ({3})" -f $status, $r.stem, $r.summary, $r.test)
}
if ($proof.failed -ne 0) { throw ("{0} window test(s) failed; logs are under C:\rustamp\out\gui-session\ (copy out with the broker copyout op)" -f $proof.failed) }

[pscustomobject]@{
    host      = $proof.computer
    user      = $proof.user
    session   = $proof.session
    screen    = $proof.screen
    ran       = $proof.ran
    failed    = $proof.failed
    results   = $proof.results
}
