# Run ONE capture job in the guest.  Allow-listed guest script.
#
# Called by the broker (`-Op guest -Script run-capture.ps1 -GuestArgs <job>`).
# The host has staged the harness, the one job file and the reference copy at
# C:\rustamp\capture (see docs\notes\vm-run-capture.ps1); results go to
# C:\rustamp\out\<job>\ and come back via the broker `copyout` op.
#
# Order inside: prove the baseline, check the display pre-gate, run one job
# through the harness entry point, hash every output. A gate failure refuses:
# non-zero exit, no success manifest, measured values in the failure record.
#
# The display SIZE step runs before this script, ordered by the host runner
# (`set-vdd-mode.ps1` via the broker: virtual display to 1024x768 or off,
# self-verified). The pre-gate below is the FINAL refusal, read again right
# before the reference binary starts, so drift between the size step and the
# job cannot produce captures that quietly fail the harness gate.
#
# Result contract: this script's result is parsed by the host runner, so it
# emits EXACTLY ONE object (the summary at the end) and nothing else on the
# output stream. The human-readable transcript goes to
# guest-transcript.txt inside the output directory instead, where it is
# hash-covered and comes back with the captures. Extra Write-Output lines
# would come back as a mixed array and break the host's field reads.

param([string]$Job = '')

$ErrorActionPreference = 'Stop'

if ([string]::IsNullOrWhiteSpace($Job)) { throw 'run-capture needs a job name (exactly one job per run)' }
if ($Job -match '[\\/]' -or $Job -notmatch '^[A-Za-z0-9._-]+$') { throw "job name '$Job' is not allowed (plain name, no path)" }

$CaptureRoot = 'C:\rustamp\capture'
$OutRoot     = 'C:\rustamp\out'
$OutDir      = Join-Path $OutRoot $Job

$transcript = New-Object System.Collections.Generic.List[string]
function Show($label, $value) { $transcript.Add(("{0,-22} {1}" -f $label, $value)) }

# --- 1. baseline facts (the host restored the checkpoint; the guest proves it) --
$os = Get-CimInstance Win32_OperatingSystem
$bootTime = $os.LastBootUpTime
Show 'job' $Job
Show 'user' $env:USERNAME
Show 'computer' $env:COMPUTERNAME
Show 'guest boot time' $bootTime.ToString('o')
Show 'os' $os.Caption

# --- 2b. sandbox-path compatibility (before anything is staged, link later) ---
# Jobs address the reference playlist by its sandbox path
# (C:\winamp\r_o4_60.m3u), which does not exist in the VM. Rather than
# rewriting job files (which must stay portable across both hosts), the guest
# links the legacy path to the staged reference copy once it is copied in
# (see below, after copyin), so every sandbox-absolute reference path keeps
# resolving. A missing playlist would leave the reference with an empty list,
# which changes dialog behaviour - the previous run's single failure
# (the Jump-to-time dialog never opened) is attributed to exactly this.

# --- 2. display pre-gate: refuse before the reference binary starts ------------
Add-Type -AssemblyName System.Windows.Forms
$logPixels = (Get-ItemProperty 'HKCU:\Control Panel\Desktop' -Name LogPixels -ErrorAction SilentlyContinue).LogPixels
$screen = [System.Windows.Forms.SystemInformation]::VirtualScreen
$screens = @([System.Windows.Forms.Screen]::AllScreens | ForEach-Object { ("{0} primary={1} bounds={2}" -f $_.DeviceName, $_.Primary, $_.Bounds) })
Show 'logpixels (want 96)' $logPixels
Show 'virtual screen (want 1024x768)' $screen.ToString()
foreach ($s in $screens) { Show 'monitor' $s }
if ($logPixels -ne 96) { throw "display pre-gate refused: LogPixels is $logPixels, want 96 (100 % scale)" }
if ($screen.Width -ne 1024 -or $screen.Height -ne 768) {
    throw ("display pre-gate refused: virtual screen is {0}x{1}, want 1024x768 (screens: {2})" -f $screen.Width, $screen.Height, ($screens -join ' | '))
}

# --- 3. one job through the harness entry point, INSIDE the interactive session -
# The broker's PowerShell Direct sessions land in session 0, which has no
# desktop: the reference binary starts there but never shows a window (its
# process lingers windowless while the harness waits 60 s and fails). So the
# harness runs the way the window tests do: a one-shot scheduled task as the
# console user with an Interactive logon, whose payload runs the bootstrap and
# always leaves a done sentinel. Session 0 then only polls for that sentinel
# and refuses loudly if the task landed anywhere but the interactive session.
$taskName = 'RustampCaptureSession'
$innerPath = Join-Path $CaptureRoot ("inner-" + $Job + ".ps1")
$innerDone = Join-Path $OutDir 'inner-done.json'

# Find the interactive session: explorer owns the console desktop, so its
# session is the interactive one and its owner is the auto-logon user.
# Fallback: `query session` looking for the Active console row. No hardcoded
# user: either signal follows whoever provisioned the guest.
$interactiveSession = $null
$interactiveUser = $null
$interactiveDomain = $null
$explorer = Get-Process -Name explorer -ErrorAction SilentlyContinue | Select-Object -First 1
if ($explorer) {
    $interactiveSession = $explorer.SessionId
    $owner = (Get-CimInstance Win32_Process -Filter ("ProcessId = " + $explorer.Id) | Invoke-CimMethod -MethodName GetOwner)
    $interactiveUser = $owner.User
    $interactiveDomain = $owner.Domain
    Show 'interactive session' ("{0} via explorer (user {1}\{2})" -f $interactiveSession, $interactiveDomain, $interactiveUser)
} else {
    Show 'interactive session' 'no explorer; falling back to `query session`'
    foreach ($line in (query session 2>$null)) {
        if ($line -match '^\s*>?\s*console\s+(\S+)\s+(\d+)\s+Active') {
            $interactiveUser = $Matches[1]
            $interactiveSession = [int]$Matches[2]
            $interactiveDomain = $env:COMPUTERNAME
            break
        }
    }
}
if ($null -eq $interactiveSession) { throw 'no interactive session found: explorer is not running and `query session` shows no Active console; the reference has no desktop to appear on' }
if ($interactiveSession -eq 0) { throw 'the "interactive" session resolved to session 0, which has no desktop; refusing instead of reporting a pass' }
if ([string]::IsNullOrWhiteSpace($interactiveUser)) { throw 'interactive session found but its user could not be resolved; refusing' }

# The payload is a fixed local path the task runs; nothing expands at write
# time except the three paths below (single-quoted here-string otherwise).
$bootstrapPath = Join-Path $CaptureRoot 'tools\vm-capture\sandbox-bootstrap.ps1'
$jobPath = Join-Path $CaptureRoot ("tools\vm-capture\jobs\" + $Job + ".json")
$installPath = 'C:\rustamp\winamp'
$referencePath = Join-Path $CaptureRoot 'winamp295'
$inner = @'
$ErrorActionPreference = 'Continue'
$InnerDone = '__INNERDONE__'
$Bootstrap = '__BOOTSTRAP__'
$JobFile = '__JOBFILE__'
$CaptureRoot = '__CAPTUREROOT__'
$OutDir = '__OUTDIR__'
$InstallRoot = '__INSTALLROOT__'
$ReferenceRoot = '__REFERENCEROOT__'

# Fail loudly in session 0: the reference cannot show a window there, and a
# pass from there would be a lie.
$me = (Get-Process -Id $PID).SessionId
$harnessExit = -1
$failure = ''
if ($me -eq 0) {
    $failure = 'refusing: the capture task landed in session 0, which has no desktop'
} else {
    # Sandbox-path compatibility: jobs address the playlist by its sandbox path
    # (C:\winamp\r_o4_60.m3u), which does not exist here. Link it to the staged
    # reference copy so every sandbox-absolute reference path resolves; a
    # missing playlist would leave the reference with an empty list, which
    # changes dialog behaviour (one dialog in an earlier run never opened).
    $legacyLink = 'C:\winamp'
    if (-not (Test-Path $legacyLink)) {
        $null = New-Item -ItemType SymbolicLink -Path $legacyLink -Target $ReferenceRoot
    }
    & powershell.exe -NoProfile -ExecutionPolicy Bypass -File $Bootstrap -Job $JobFile -NoLogoff `
        -ResearchRoot $CaptureRoot `
        -ReferenceRoot $ReferenceRoot `
        -InstallRoot $InstallRoot `
        -OutDir $OutDir
    $harnessExit = $LASTEXITCODE
    if ($harnessExit -ne 0) { $failure = ("harness exit {0}" -f $harnessExit) }
}
@{ session = $me; user = $env:USERNAME; computer = $env:COMPUTERNAME; harnessExit = $harnessExit; failure = $failure } |
    ConvertTo-Json | Set-Content -Path $InnerDone -Encoding UTF8
exit 0
'@
$inner = $inner.Replace('__INNERDONE__', $innerDone).Replace('__BOOTSTRAP__', $bootstrapPath).Replace('__JOBFILE__', $jobPath).Replace('__CAPTUREROOT__', $CaptureRoot).Replace('__OUTDIR__', $OutDir).Replace('__INSTALLROOT__', $installPath).Replace('__REFERENCEROOT__', $referencePath)
$inner | Set-Content -Path $innerPath -Encoding UTF8

Unregister-ScheduledTask -TaskName $taskName -Confirm:$false -ErrorAction SilentlyContinue
$action = New-ScheduledTaskAction -Execute 'powershell.exe' `
    -Argument ('-NoProfile -NonInteractive -ExecutionPolicy Bypass -File "{0}"' -f $innerPath)
$principal = New-ScheduledTaskPrincipal -UserId ("{0}\{1}" -f $interactiveDomain, $interactiveUser) `
    -LogonType Interactive -RunLevel Limited
$trigger = New-ScheduledTaskTrigger -Once -At (Get-Date).AddMinutes(1)
Register-ScheduledTask -TaskName $taskName -Action $action -Principal $principal `
    -Trigger $trigger -Force | Out-Null
Show 'capture task' ("registered '{0}' as {1}\{2} (Interactive, session {3})" -f $taskName, $interactiveDomain, $interactiveUser, $interactiveSession)
Start-ScheduledTask -TaskName $taskName
$deadline = (Get-Date).AddMinutes(30)
$innerDoneObj = $null
while ((Get-Date) -lt $deadline) {
    Start-Sleep -Seconds 5
    if (Test-Path $innerDone) {
        try { $innerDoneObj = Get-Content -Path $innerDone -Raw | ConvertFrom-Json } catch { continue }
        break
    }
}
Unregister-ScheduledTask -TaskName $taskName -Confirm:$false -ErrorAction SilentlyContinue
if (-not $innerDoneObj) { throw 'the capture task produced no done sentinel within 30 minutes; refusing instead of reporting a pass' }
if ($innerDoneObj.session -eq 0) { throw ("the capture task ran in session 0 ({0}); refusing" -f $innerDoneObj.failure) }
if ($innerDoneObj.session -ne $interactiveSession) { throw ("the capture task ran in session {0}, not the interactive session {1}; refusing" -f $innerDoneObj.session, $interactiveSession) }
$harnessExit = [int]$innerDoneObj.harnessExit
Show 'harness exit' $harnessExit
Show 'harness session' ("{0} as {1}" -f $innerDoneObj.session, $innerDoneObj.user)
# The transcript is written before the refusal below, so a later diagnostic
# pass over the guest can still read what this run saw.
$transcript | Set-Content -Path (Join-Path $OutDir 'guest-transcript.txt') -Encoding UTF8
if ($harnessExit -ne 0) { throw "harness refused or failed with exit $harnessExit (see its failure record; nothing is promoted)" }

# --- 4. hash every output (the host re-verifies these after copyout) -----------
$hashes = [ordered]@{}
foreach ($f in Get-ChildItem -Path $OutDir -File -Recurse | Sort-Object FullName) {
    $rel = $f.FullName.Substring($OutDir.Length + 1)
    $hashes[$rel] = (Get-FileHash -Path $f.FullName -Algorithm SHA256).Hash
}
$hashes | ConvertTo-Json | Set-Content -Path (Join-Path $OutDir 'hashes.json') -Encoding UTF8

[pscustomobject]@{
    job        = $Job
    user       = $env:USERNAME
    host       = $env:COMPUTERNAME
    bootTime   = $bootTime.ToString('o')
    logPixels  = $logPixels
    screen     = $screen.ToString()
    screens    = @($screens)
    harnessExit = $harnessExit
    files      = $hashes.Count
    outDir     = $OutDir
}
