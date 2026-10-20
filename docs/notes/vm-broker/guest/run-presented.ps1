# Run ONE presented capture job in the guest. Allow-listed guest script.
#
# Called by the broker (`-Op guest -Script run-presented.ps1 -GuestArgs <job>`).
# The host has staged the harness, the one job file, our built tree and the
# reference skin at C:\rustamp\capture (see docs\notes\vm-run-presented.ps1);
# results go to C:\rustamp\out\<job>\ and come back via the broker `copyout` op.
#
# Order inside: prove the baseline, check the display pre-gate, launch OUR
# player in the INTERACTIVE console session (one-shot scheduled task as the
# console user with an Interactive logon, then poll for a done sentinel from
# session 0; refuse loudly if the task landed anywhere but an interactive
# session), open the equalizer and the playlist with WM_COMMAND 40036 and
# 40040 (the recorded 1:1 landmark of the reference has both windows open and
# the comparison must start from the same state), run each step through the
# harness Invoke-Capture.ps1 with the same fields the reference jobs use, write
# hashes.json and DONE.json with per-step results plus stdout/stderr of every
# harness call, emit exactly ONE object on stdout (the summary), and put the
# human transcript in a file.

param([string]$Job = '')

$ErrorActionPreference = 'Stop'

if ([string]::IsNullOrWhiteSpace($Job)) { throw 'run-presented needs a job name (exactly one job per run)' }
if ($Job -match '[\\/]' -or $Job -notmatch '^[A-Za-z0-9._-]+$') { throw "job name '$Job' is not allowed (plain name, no path)" }

$CaptureRoot = 'C:\rustamp\capture'
$OutRoot     = 'C:\rustamp\out'
$OutDir      = Join-Path $OutRoot $Job

# The interactive payload writes its done sentinel and the per-step logs into
# $OutDir, and the session-0 waiter polls a file there. The directory must exist
# before the task starts: the first version never created it, so a task that ran
# perfectly had nowhere to leave evidence and the run looked like a hang.
$null = New-Item -ItemType Directory -Force -Path $OutDir

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

# --- 2. display pre-gate: refuse before our player starts -----------------------
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

# --- 3. launch our player in the interactive session, then drive it -----------
# The broker's PowerShell Direct sessions land in session 0, which has no
# desktop: our player starts there but never shows a window. So we run the
# same way the window tests do: a one-shot scheduled task as the console user
# with an Interactive logon, whose payload launches the player, opens EQ+PL,
# runs the harness steps, and always leaves a done sentinel. Session 0 then
# only polls for that sentinel and refuses loudly if the task landed anywhere
# but the interactive session.
$taskName = 'RustampPresentedSession'
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
if ($null -eq $interactiveSession) { throw 'no interactive session found: explorer is not running and `query session` shows no Active console; our player has no desktop to appear on' }
if ($interactiveSession -eq 0) { throw 'the "interactive" session resolved to session 0, which has no desktop; refusing instead of reporting a pass' }
if ([string]::IsNullOrWhiteSpace($interactiveUser)) { throw 'interactive session found but its user could not be resolved; refusing' }

# Paths for the inner payload
$jobPath = Join-Path $CaptureRoot ("tools\vm-capture\jobs\" + $Job + ".json")
$installRoot = Join-Path $CaptureRoot 'built'   # the host stages target\debug there (C:\rustamp\capture\built)
$skinPath = Join-Path $CaptureRoot 'skins\base-2.91.wsz'
$configPath = Join-Path $OutDir 'rustamp.toml'
$captureScript = Join-Path $CaptureRoot 'tools\vm-capture\Invoke-Capture.ps1'
$harnessModule = Join-Path $CaptureRoot 'tools\vm-capture\RustampCapture.psm1'

# The inner payload runs in the interactive session. It launches our player
# with the required arguments, opens EQ and PL via WM_COMMAND, then runs each
# job step through Invoke-Capture.ps1.
$inner = @'
$ErrorActionPreference = 'Continue'
$InnerDone = '__INNERDONE__'
$JobFile = '__JOBFILE__'
$CaptureRoot = '__CAPTUREROOT__'
$OutDir = '__OUTDIR__'
$InstallRoot = '__INSTALLROOT__'
$SkinPath = '__SKINPATH__'
$ConfigPath = '__CONFIGPATH__'
$CaptureScript = '__CAPTURESCRIPT__'
$HarnessModule = '__HARNESSMODULE__'

# Fail loudly in session 0: our player cannot show a window there, and a
# pass from there would be a lie.
$me = (Get-Process -Id $PID).SessionId
$harnessExit = -1
$failure = ''
$stepOutputs = @()
$stdoutStderr = @()

if ($me -eq 0) {
    $failure = 'refusing: the presented task landed in session 0, which has no desktop'
} else {
    Import-Module $HarnessModule -Force

    # Always leave the sentinel. Without this, a terminating error anywhere in
    # this payload (a window that never appears, a player that will not start,
    # a harness call that throws) left session 0 polling for six minutes and
    # then reporting "no done sentinel" - the failure said nothing about what
    # went wrong. A trap catches that at any point and still writes the record,
    # including the window list, which is what tells a missing class apart from
    # a window that was never created.
    trap {
        $message = ($_ | Out-String).Trim()
        $windows = @()
        try {
            $rustampPids = @(Get-Process -Name rustamp -ErrorAction SilentlyContinue | ForEach-Object { $_.Id })
            $windows = @([RustampCapture.Native]::EnumTopLevelWindows() |
                Where-Object { $rustampPids -contains [int]$_.Pid } |
                ForEach-Object { "{0:X} class='{1}' title='{2}' visible={3} pid={4}" -f [int64]$_.Hwnd, $_.ClassName, $_.Title, $_.Visible, $_.Pid })
        } catch { }
        $record = [ordered]@{
            session      = $me
            user         = $env:USERNAME
            computer     = $env:COMPUTERNAME
            harnessExit  = -1
            failure      = $message
            steps        = @($stepOutputs)
            stdoutStderr = @($stdoutStderr) + @('trap: ' + $message) + @($windows)
            windows      = $windows
        }
        $record | ConvertTo-Json -Depth 5 | Set-Content -Path $InnerDone -Encoding UTF8
        exit 1
    }

    # Parse the job file to get steps, winamp_args, expect_client_size
    $jobRaw = Get-Content -Path $JobFile -Raw -Encoding UTF8
    $job = $jobRaw | ConvertFrom-Json
    $winampArgs = if ($job.winamp_args) { $job.winamp_args } else { '/NOREG' }
    $jobExpectSize = if ($job.expect_client_size) { $job.expect_client_size } else { $null }

    # Launch our player with the required arguments
    # --no-audio --skin <skin> --volume 0 /NOREG /CONFIG=<config>
    $ourArgs = "--no-audio --skin `"$SkinPath`" --volume 0 $winampArgs /CONFIG=`"$ConfigPath`""
    Write-Host "starting rustamp.exe $ourArgs (cwd $InstallRoot)"
    $proc = Start-Process -FilePath (Join-Path $InstallRoot 'rustamp.exe') -ArgumentList $ourArgs -WorkingDirectory $InstallRoot -PassThru

    # Wait for the main window (Class 'Winamp v1.x', process 'rustamp')
    $deadline = (Get-Date).AddSeconds(60)
    $mainHwnd = $null
    while ((Get-Date) -lt $deadline) {
        try {
            $t = Resolve-CaptureWindow -Class 'Winamp v1.x' -ProcessName 'rustamp'
            $mainHwnd = $t.Hwnd
            Write-Host ("main window is up: {0} pid={1} client={2}x{3}" -f $t.HwndHex, $t.Pid, $t.ClientWidth, $t.ClientHeight)
            break
        }
        catch { Start-Sleep -Milliseconds 500 }
    }
    if (-not $mainHwnd) { throw "no 'Winamp v1.x' window for rustamp.exe within 60 s" }

    # Open the equalizer (WM_COMMAND 40036) and playlist (WM_COMMAND 40040).
    # The recorded 1:1 landmark of the reference has both windows open and the
    # comparison must start from the same state.
    #
    # A single post is not enough: our player installs its WM_COMMAND hook with
    # the first UI tick, so a message posted the instant the window exists is
    # dropped - six presented runs failed that way (main-inactive, eq-*), each
    # one waiting for a window that never opened. Post, then wait for the window
    # to exist, and post again only while it is still absent (the command is a
    # toggle, so it must never be posted twice after the window appeared).
    $n = [RustampCapture.Native]
    $WM_COMMAND = 273
    $EQ_CMD = 40036
    $PL_CMD = 40040

    # The harness's Native type has no PostMessage (the first version of this
    # runner called one anyway; with $ErrorActionPreference = 'Continue' the
    # error scrolled past and the equalizer simply never opened, so every
    # `eq-*` capture waited for a window that did not exist). Declare the one
    # call this needs under our own type name so it cannot collide with the
    # module's.
    if (-not ('RustampPresented.User32' -as [type])) {
        Add-Type -Namespace RustampPresented -Name User32 -MemberDefinition '[System.Runtime.InteropServices.DllImport("user32.dll", SetLastError = true)] public static extern bool PostMessageW(System.IntPtr hWnd, uint msg, System.IntPtr wParam, System.IntPtr lParam);'
    }

    function Open-WindowByCommand {
        param([string]$Label, [int]$Command, [string]$Class, [int]$Seconds = 20)
        $attempts = 0
        $deadline = (Get-Date).AddSeconds($Seconds)
        while ((Get-Date) -lt $deadline) {
            $present = $false
            try { $null = Resolve-CaptureWindow -Class $Class -ProcessName 'rustamp'; $present = $true } catch { $present = $false }
            if ($present) {
                Write-Host ("{0} window present after {1} post(s): {2}" -f $Label, $attempts, $Class)
                return $true
            }
            $attempts++
            Write-Host ("posting WM_COMMAND {0} ({1}) to the main window, attempt {2}" -f $Command, $Label, $attempts)
            [void][RustampPresented.User32]::PostMessageW($mainHwnd, [uint32]$WM_COMMAND, [IntPtr]$Command, [IntPtr]::Zero)
            Start-Sleep -Milliseconds 700
        }
        Write-Host ("{0} window still absent after {1} post(s) in {2} s (class '{3}')" -f $Label, $attempts, $Seconds, $Class)
        return $false
    }

    $eqOpened = Open-WindowByCommand -Label 'equalizer' -Command $EQ_CMD -Class 'Winamp EQ'
    $plOpened = Open-WindowByCommand -Label 'playlist' -Command $PL_CMD -Class 'Winamp PE'
    if (-not ($eqOpened -and $plOpened)) {
        throw ("the start state could not be reached: equalizer={0}, playlist={1} - every capture would be of the wrong state" -f $eqOpened, $plOpened)
    }

    # Run each step through Invoke-Capture.ps1
    foreach ($step in $job.steps) {
        $stepName = $step.name
        $window = if ($step.window) { $step.window } else { 'Winamp v1.x' }
        $inputMethod = if ($step.input_method) { $step.input_method } else { 'real-mouse' }
        $captureName = if ($step.capture) { $step.capture } else { "$stepName.png" }
        # Replace .reference.png with .presented.png in the capture name
        $captureName = $captureName -replace '\.reference\.png$', '.presented.png'
        $outPath = Join-Path $OutDir $captureName
        $action = if ($step.action) { $step.action } else { '' }
        $down = if ($step.down) { $step.down } else { '' }
        $move = if ($step.move) { $step.move } else { '' }
        $up = if ($step.up) { $step.up } else { '' }
        $msg = if ($step.msg) { $step.msg } else { 0 }
        $wparam = if ($step.wparam) { $step.wparam } else { 0 }
        $lparam = if ($step.lparam) { $step.lparam } else { 0 }
        $expectReply = if ($step.expect_reply) { $step.expect_reply } else { $null }
        $expectSize = if ($step.expect_client_size) { $step.expect_client_size } else { $jobExpectSize }
        $mayChange = $false
        foreach ($fname in @('may_change_client_size','may_change_size','changes_client_size','may_resize','resizes_window','changes_size','allow_client_size_change','client_size_may_change')) {
            $v = $step.$fname
            if ($null -ne $v -and -not [string]::IsNullOrWhiteSpace([string]$v)) {
                $txt = [string]$v
                if ($txt -in @('True','true','yes','1','on')) { $mayChange = $true }
            }
        }
        if (-not $mayChange -and ($stepName -match '_act$')) { $mayChange = $true }

        $harnessArgs = @(
            '-InputMethod', $inputMethod,
            '-Out', $outPath,
            '-Class', $window,
            '-ProcessName', 'rustamp'
        )
        if ($action) { $harnessArgs += '-Action', $action }
        if ($down) { $harnessArgs += '-Down', $down }
        if ($move) { $harnessArgs += '-Move', $move }
        if ($up) { $harnessArgs += '-Up', $up }
        if ($msg -ne 0) { $harnessArgs += '-Msg', $msg }
        if ($wparam -ne 0) { $harnessArgs += '-WParam', $wparam }
        if ($lparam -ne 0) { $harnessArgs += '-LParam', $lparam }
        if ($null -ne $expectReply) { $harnessArgs += '-ExpectReply', $expectReply }
        if ($expectSize -and $expectSize -ne 'none') { $harnessArgs += '-ExpectClientSize', $expectSize }
        if ($mayChange) { $harnessArgs += '-MayChangeClientSize' }
        # Manifest file is shared across all steps
        $harnessArgs += '-Manifest', (Join-Path $OutDir 'manifest.jsonl')

        Write-Host ("running step '$stepName' -> $captureName")
        Write-Host ("  harness args: $($harnessArgs -join ' ')")
        $logBase = Join-Path $OutDir ("step-$stepName")
        # Start-Process, not the call operator: the redirection parameters
        # belong to Start-Process, and passing them to powershell.exe (the first
        # version did exactly that) made the child fail before it ran a step,
        # with an empty $LASTEXITCODE and no output at all.
        #
        # Arguments must be quoted by hand: Start-Process joins an array with
        # spaces and would split an action text like "the landmark start state,
        # main window at its normal size" into eight arguments.
        $quote = { param($a) '"' + ([string]$a -replace '"', '\"') + '"' }
        $allArgs = @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', $CaptureScript) + $harnessArgs
        $argLine = (($allArgs | ForEach-Object { & $quote $_ }) -join ' ')
        $result = Start-Process -FilePath 'powershell.exe' -ArgumentList $argLine `
            -RedirectStandardOutput ($logBase + '.stdout.txt') `
            -RedirectStandardError ($logBase + '.stderr.txt') `
            -WindowStyle Hidden -Wait -PassThru
        $exitCode = $result.ExitCode
        $stdout = if (Test-Path ($logBase + '.stdout.txt')) { Get-Content -Path ($logBase + '.stdout.txt') -Raw } else { '' }
        $stderr = if (Test-Path ($logBase + '.stderr.txt')) { Get-Content -Path ($logBase + '.stderr.txt') -Raw } else { '' }
        $stepOutputs += [pscustomobject]@{
            step = $stepName
            capture = $captureName
            exitCode = $exitCode
            stdout = $stdout
            stderr = $stderr
        }
        $stdoutStderr += "=== STEP $stepName (exit $exitCode) ==="
        $stdoutStderr += $stdout
        $stdoutStderr += $stderr
        if ($exitCode -ne 0) {
            $failure = "step '$stepName' failed with exit $exitCode"
            break
        }
    }
    $harnessExit = if ($failure) { 1 } else { 0 }
}

@{ session = $me; user = $env:USERNAME; computer = $env:COMPUTERNAME; harnessExit = $harnessExit; failure = $failure; steps = $stepOutputs; stdoutStderr = $stdoutStderr } |
    ConvertTo-Json -Depth 5 | Set-Content -Path $InnerDone -Encoding UTF8
exit 0
'@
$inner = $inner.Replace('__INNERDONE__', $innerDone).Replace('__JOBFILE__', $jobPath).Replace('__CAPTUREROOT__', $CaptureRoot).Replace('__OUTDIR__', $OutDir).Replace('__INSTALLROOT__', $installRoot).Replace('__SKINPATH__', $skinPath).Replace('__CONFIGPATH__', $configPath).Replace('__CAPTURESCRIPT__', $captureScript).Replace('__HARNESSMODULE__', $harnessModule)
$inner | Set-Content -Path $innerPath -Encoding UTF8

Unregister-ScheduledTask -TaskName $taskName -Confirm:$false -ErrorAction SilentlyContinue
$action = New-ScheduledTaskAction -Execute 'powershell.exe' `
    -Argument ('-NoProfile -NonInteractive -ExecutionPolicy Bypass -File "{0}"' -f $innerPath)
$principal = New-ScheduledTaskPrincipal -UserId ("{0}\{1}" -f $interactiveDomain, $interactiveUser) `
    -LogonType Interactive -RunLevel Limited
$trigger = New-ScheduledTaskTrigger -Once -At (Get-Date).AddMinutes(1)
Register-ScheduledTask -TaskName $taskName -Action $action -Principal $principal `
    -Trigger $trigger -Force | Out-Null
Show 'presented task' ("registered '{0}' as {1}\{2} (Interactive, session {3})" -f $taskName, $interactiveDomain, $interactiveUser, $interactiveSession)
Start-ScheduledTask -TaskName $taskName
$deadline = (Get-Date).AddMinutes(6)
$innerDoneObj = $null
while ((Get-Date) -lt $deadline) {
    Start-Sleep -Seconds 5
    if (Test-Path $innerDone) {
        try { $innerDoneObj = Get-Content -Path $innerDone -Raw | ConvertFrom-Json } catch { continue }
        break
    }
}
Unregister-ScheduledTask -TaskName $taskName -Confirm:$false -ErrorAction SilentlyContinue
if (-not $innerDoneObj) { throw 'the presented task produced no done sentinel within 6 minutes; refusing instead of reporting a pass' }
if ($innerDoneObj.session -eq 0) { throw ("the presented task ran in session 0 ({0}); refusing" -f $innerDoneObj.failure) }
if ($innerDoneObj.session -ne $interactiveSession) { throw ("the presented task ran in session {0}, not the interactive session {1}; refusing" -f $innerDoneObj.session, $interactiveSession) }
$harnessExit = [int]$innerDoneObj.harnessExit
Show 'harness exit' $harnessExit
Show 'harness session' ("{0} as {1}" -f $innerDoneObj.session, $innerDoneObj.user)

# Write the human transcript (includes stdout/stderr of every harness call)
$transcript.Add('--- STEP STDOUT/STDERR ---')
if ($innerDoneObj.stdoutStderr) { $transcript.Add($innerDoneObj.stdoutStderr) }
$transcript | Set-Content -Path (Join-Path $OutDir 'guest-transcript.txt') -Encoding UTF8
if ($harnessExit -ne 0) { throw "harness refused or failed with exit $harnessExit (see its failure record; nothing is promoted)" }

# --- 4. hash every output (the host re-verifies these after copyout) -----------
$hashes = [ordered]@{}
foreach ($f in Get-ChildItem -Path $OutDir -File -Recurse | Sort-Object FullName) {
    $rel = $f.FullName.Substring($OutDir.Length + 1)
    $hashes[$rel] = (Get-FileHash -Path $f.FullName -Algorithm SHA256).Hash
}
$hashes | ConvertTo-Json | Set-Content -Path (Join-Path $OutDir 'hashes.json') -Encoding UTF8

# --- 5. write DONE.json with per-step results ----------------------------------
$done = [ordered]@{
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
    steps      = $innerDoneObj.steps
}
$done | ConvertTo-Json -Depth 5 | Set-Content -Path (Join-Path $OutDir 'DONE.json') -Encoding UTF8

# --- 6. emit exactly ONE summary object on stdout -------------------------------
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