# Run the PACKAGED player smoke test INSIDE the guest's interactive session.
#
# Allow-listed guest script. Called by the broker exactly like run-tests.ps1:
#
#   broker -Op guest -Script run-smoke.ps1
#
# No broker arguments: every path is a fixed local path below (the staging
# layout `docs/notes/vm-run-smoke.ps1` copies in), so the allow-list model
# holds trivially.
#
# Why this script exists: the broker's PowerShell Direct sessions land in
# session 0 as the S4U user, which has no desktop. The packaged player is a
# winit GUI program: starting it in session 0 panics inside winit before any
# window exists. This script therefore follows the `run-gui-session.ps1`
# precedent: it discovers the auto-logon interactive session (explorer owns
# the console desktop; its session is the interactive one), registers a
# one-shot Scheduled Task as that user with LogonType Interactive (i.e. on
# the console desktop), and waits for it. The payload is a fixed local path
# written by THIS allow-listed script, never an argument, so nothing the
# broker passes in can change what runs.
#
# What the inner payload proves, in order (each step fails loudly):
#   1. the packaged `player\rustamp.exe` starts with `--no-audio` (the silent
#      windowed mode: a real window, the null output, no audio device needed)
#      and stays up;
#   2. its main window (exact title `Rustamp`, owned by the player pid,
#      visible) appears within a bounded wait, at the S01 rest client size;
#   3. the bundled `rustamp-plughost32.exe` (staged beside the player, RFC 0006
#      section 8 packaged layout) appears as a live child of the player, i.e.
#      the supervisor spawned it and it survived startup far enough to serve
#      the framed stdio control channel (startup `Hello`, per-`Ping` `Pong`);
#   4. the existing capture tool (`cargo xtask capture`, staged alongside,
#      NOT rebuilt here) screenshots a `Rustamp*` window to PNG;
#   5. the player's own stdout names the silent path
#      (`output null (windowed; silent, no audio device)`), not a real device.
#
# Results land in `C:\rustamp\out\smoke\` and come back via the existing
# broker `copyout` op (source under `C:\rustamp\out`, as that op requires).
# The host runner (`docs/notes/vm-run-smoke.ps1`) re-verifies every file,
# including a SHA-256 recompute against the guest-written `hashes.json`.
#
# Result contract (runbook section 7): this script emits EXACTLY ONE object
# (the proof, at the end) and nothing else on the output stream. Progress
# lives in `guest-transcript.txt` inside the outputs, which is hash-covered
# and comes back with the captures. Extra Write-Output lines would come back
# as a mixed array and break the host's field reads.
#
# Needs no desktop of its own for the outer half: session discovery inspects
# system-wide state, so running this script from the broker's session-0
# PowerShell Direct session is fine. It proves the session; only the inner
# payload uses it.

$ErrorActionPreference = 'Stop'

$TaskName  = 'RustampSmoke'
$SmokeRoot = 'C:\rustamp\smoke'
$OutDir    = 'C:\rustamp\out\smoke'
$InnerPath = Join-Path $OutDir 'inner.ps1'
$DonePath  = Join-Path $OutDir 'done.json'
$ProofPath = Join-Path $OutDir 'proof.json'

function Session-Of([int]$ProcessId) {
    (Get-Process -Id $ProcessId -ErrorAction Stop).SessionId
}

# --- 0. record where THIS script landed (must be session 0; informational) ---
# (Kept in the transcript, not on stdout: the result contract allows exactly
# one output object.)
$transcript = New-Object System.Collections.Generic.List[string]
function Show($label, $value) { $transcript.Add(("{0,-22} {1}" -f $label, $value)) }
Show 'launcher session' (Session-Of $PID)
Show 'launcher user' ("{0} on {1}" -f $env:USERNAME, $env:COMPUTERNAME)

# --- 1. the staged smoke tree must be there (the host runner copied it) -----
$playerExe = Join-Path $SmokeRoot 'player\rustamp.exe'
$hostExe   = Join-Path $SmokeRoot 'player\rustamp-plughost32.exe'
$toolExe    = Join-Path $SmokeRoot 'tools\xtask.exe'
$fixture    = Join-Path $SmokeRoot 'media\sine.flac'
foreach ($need in @($playerExe, $hostExe, $toolExe, $fixture)) {
    if (-not (Test-Path $need)) { throw "smoke tree incomplete: missing $need (the host runner did not stage the package)" }
}
Show 'player' $playerExe
Show 'bundled host' $hostExe
Show 'capture tool' $toolExe
Show 'fixture' $fixture

# --- 2. find the interactive session and its user -----------------------------
# Same signals as run-gui-session.ps1: explorer owns the console desktop, so
# its session is the interactive one and its owner is the auto-logon user.
# Fallback: `query session` looking for the Active console row. No hardcoded
# user name: the provision script sets auto-logon to whoever provisioned, and
# either signal follows it.
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
        # Active console row looks like: ">console  rustamp  1  Active"
        if ($line -match '^\s*>?\s*console\s+(\S+)\s+(\d+)\s+Active') {
            $interactiveUser = $Matches[1]
            $interactiveSession = [int]$Matches[2]
            $interactiveDomain = $env:COMPUTERNAME
            break
        }
    }
}
if ($null -eq $interactiveSession) { throw 'no interactive session found: explorer is not running and `query session` shows no Active console; the player has no desktop to open on' }
if ($interactiveSession -eq 0) { throw 'the "interactive" session resolved to session 0, which has no desktop; refusing instead of reporting a pass' }
if ([string]::IsNullOrWhiteSpace($interactiveUser)) { throw 'interactive session found but its user could not be resolved; refusing' }

# --- 3. write the inner payload (fixed local path, values baked in) -----------
# The only broker-controlled input to this whole script is nothing at all, so
# the payload below is fully fixed except for the paths baked in here by
# .Replace (single-quoted here-string: nothing expands at write time).
if (Test-Path $OutDir) { Remove-Item -Recurse -Force $OutDir }
$null = New-Item -ItemType Directory -Force -Path $OutDir

$inner = @'
$ErrorActionPreference = 'Stop'
$PlayerExe = '__PLAYER__'
$Fixture   = '__FIXTURE__'
$ToolExe   = '__TOOL__'
$OutDir    = '__OUTDIR__'
$ProofPath = Join-Path $OutDir 'proof.json'
$DonePath  = Join-Path $OutDir 'done.json'
$PngPath   = Join-Path $OutDir 'player.png'
$StdoutLog = Join-Path $OutDir 'player.stdout.log'
$StderrLog = Join-Path $OutDir 'player.stderr.log'
$Transcript = Join-Path $OutDir 'guest-transcript.txt'

$log = New-Object System.Collections.Generic.List[string]
function Say($label, $value) { $log.Add(("{0,-22} {1}" -f $label, $value)) }

function Finish-Failure([string]$step, [string]$detail) {
    # Every failure proof carries the seen session: without it the outer
    # session check formats an empty value (`ran in session , ...`) and masks
    # the real step failure. Looked up dynamically so this works no matter
    # which step calls it.
    $sess = $null
    try { $sess = (Get-Process -Id $PID -ErrorAction Stop).SessionId } catch { $sess = $null }
    Say ("STEP FAILED: " + $step) $detail
    Say 'inner session at failure' $sess
    $log | Set-Content -Path $Transcript -Encoding UTF8
    [pscustomobject]@{
        ok      = $false
        session = $sess
        failed  = $step
        detail  = $detail
    } | ConvertTo-Json -Depth 6 | Set-Content -Path $ProofPath -Encoding UTF8
    @{ ok = $false; failed = $step } | ConvertTo-Json | Set-Content -Path $DonePath -Encoding UTF8
    exit 3
}

try {
    # Fail loudly in session 0: no desktop there, and a pass from there would
    # be a lie (same refusal as run-gui-session.ps1's inner script).
    $me = (Get-Process -Id $PID).SessionId
    Say 'inner session' $me
    if ($me -eq 0) { Finish-Failure 'session' 'the smoke task landed in session 0, which has no desktop; a window cannot open here' }

    Add-Type -AssemblyName System.Windows.Forms
    $screen = [System.Windows.Forms.SystemInformation]::VirtualScreen
    Say 'virtual screen' $screen.ToString()
    if ($screen.Width -le 0 -or $screen.Height -le 0) { Finish-Failure 'session' 'no usable desktop in this session (virtual screen is empty)' }

    # Isolated config: the smoke run must not read or write the guest user's
    # real configuration. (Absence is safe: the app falls back to defaults.)
    $configPath = Join-Path $OutDir 'rustamp.toml'
    Say 'config override' $configPath

    # --- start the packaged player (silent windowed mode, repo fixture) ----
    # CREATE_NO_WINDOW via ProcessStartInfo (the repo's own no_window
    # discipline): a console-subsystem child would otherwise raise a console
    # window of its own, whose title contains "rustamp.exe" and would match
    # the capture title filter below. Stdout/stderr go to files: the player
    # prints its silent-path line there while it runs (Rust stdout is
    # line-buffered even when redirected, so the line is readable pre-exit).
    # --seconds 120 is a backstop only: the script kills the player after the
    # capture, long before the limit.
    Add-Type -TypeDefinition @"
using System;
using System.Runtime.InteropServices;
using System.Text;
public class SmokeWin {
    public delegate bool CB(IntPtr h, IntPtr p);
    [DllImport("user32.dll")] public static extern bool EnumWindows(CB cb, IntPtr p);
    [DllImport("user32.dll")] public static extern int GetWindowTextW(IntPtr h, StringBuilder s, int n);
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
    [DllImport("user32.dll")] public static extern bool GetClientRect(IntPtr h, out RECT r);
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int left; public int top; public int right; public int bottom; }
    public static string ClientSize(IntPtr h) {
        RECT r; if (!GetClientRect(h, out r)) return "";
        return (r.right - r.left).ToString() + "x" + (r.bottom - r.top).ToString();
    }
    public static string WindowRect(IntPtr h) {
        RECT r; if (!GetWindowRect(h, out r)) return "";
        return "(" + r.left + "," + r.top + ")-(" + r.right + "," + r.bottom + ")";
    }
}
"@
    $psi = New-Object Diagnostics.ProcessStartInfo
    $psi.FileName = $PlayerExe
    $psi.Arguments = '--no-audio "' + $Fixture + '" --seconds 120'
    $psi.WorkingDirectory = (Split-Path $PlayerExe)
    $psi.UseShellExecute = $false
    $psi.CreateNoWindow = $true
    $psi.RedirectStandardOutput = $true
    $psi.RedirectStandardError = $true
    $psi.EnvironmentVariables['RUSTAMP_CONFIG'] = $configPath
    $psi.EnvironmentVariables.Remove('RUSTAMP_NO_GUI')
    $player = [Diagnostics.Process]::Start($psi)
    if (-not $player) { Finish-Failure 'start' 'Process.Start returned nothing for the packaged player' }
    Say 'player pid' $player.Id
    $startedAt = Get-Date

    # If the process is already gone, the packaged player does not start.
    # 0xC0000135 (-1073741515) is the known missing-runtime death; the
    # package ships static-CRT binaries, so that code here points at the
    # staged capture tool's DLLs or a broken package, not at the host.
    Start-Sleep -Seconds 2
    if ($player.HasExited) {
        $code = $player.ExitCode
        Finish-Failure 'start' ("the packaged player exited immediately with code {0} (0x{1:X}); it never reached the window" -f $code, $code)
    }
    Say 'player alive after 2 s' 'yes'

    # --- bounded wait for the exact main window (mirrors the headless test) --
    # Exact title `Rustamp`, owned by the player pid, visible. The playlist
    # window (`Rustamp Playlist`) must NOT satisfy this wait: proving the
    # main window opened is the point. 60 s budget, 100 ms poll.
    $deadline = (Get-Date).AddSeconds(60)
    $mainHwnd = [Int64]0
    $mainClient = $null
    $mainRect = $null
    while ((Get-Date) -lt $deadline) {
        if ($player.HasExited) {
            Finish-Failure 'window' 'the packaged player died while its window was awaited; no window can appear now'
        }
        $found = New-Object System.Collections.ArrayList
        [void][SmokeWin]::EnumWindows({ param($h, $p)
            $sb = New-Object Text.StringBuilder 256
            [void][SmokeWin]::GetWindowTextW($h, $sb, 256)
            $t = $sb.ToString()
            if ($t -ne 'Rustamp') { return $true }
            $childPid = 0; [void][SmokeWin]::GetWindowThreadProcessId($h, [ref]$childPid)
            if ($childPid -ne $player.Id) { return $true }
            if (-not [SmokeWin]::IsWindowVisible($h)) { return $true }
            [void]$found.Add($h)
            return $true
        }, [IntPtr]::Zero)
        if ($found.Count -ge 1) {
            $mainHwnd = [Int64]$found[0]
            Say 'main window matches' $found.Count
            $mainClient = [SmokeWin]::ClientSize([IntPtr]$mainHwnd)
            $mainRect = [SmokeWin]::WindowRect([IntPtr]$mainHwnd)
            Say 'main window hwnd' ("0x{0:X}" -f $mainHwnd)
            Say 'main window client' $mainClient
            Say 'main window rect' $mainRect
            break
        }
        Start-Sleep -Milliseconds 100
    }
    if ($mainHwnd -eq 0) { Finish-Failure 'window' 'no visible main window (exact title `Rustamp`, owned by the player) appeared within 60 s' }

    # The S01 rest client size, pinned on the packaged build too.
    if ($mainClient -ne '275x116') { Finish-Failure 'window' ("the main window client is {0}, want 275x116 (S01 rest size)" -f $mainClient) }

    # --- the bundled host must be alive as the player's child ----------------
    # The supervisor resolves the host beside rustamp.exe first (RFC 0006
    # section 8 packaged layout) and spawns it eagerly at startup; the host
    # answers the framed stdio control channel (startup Hello, per-Ping
    # Pong) only while its UI pump runs. A live child therefore proves the
    # pair started far enough to serve the handshake. Pipe bytes themselves
    # are the supervisor's private channel and are not independently read
    # here; the card states that residual gap.
    $hostPid = $null
    for ($i = 0; $i -lt 5; $i++) {
        $kids = @(Get-CimInstance Win32_Process -Filter ("ParentProcessId = " + $player.Id) -ErrorAction SilentlyContinue |
            Where-Object { $_.Name -ieq 'rustamp-plughost32.exe' })
        if ($kids.Count -ge 1) {
            $hostPid = $kids[0].ProcessId
            Say 'host child pid' $hostPid
            break
        }
        Start-Sleep -Seconds 2
    }
    if ($null -eq $hostPid) { Finish-Failure 'resident-handshake' 'no live rustamp-plughost32.exe child of the player appeared within ~10 s; the bundled host is not supervising' }

    # --- capture through the existing capture tool ----------------------------
    $listRaw = & $ToolExe capture --list 2>&1 | Out-String
    $listRaw | Set-Content -Path (Join-Path $OutDir 'window-list.log') -Encoding UTF8
    Say 'capture --list exit' $LASTEXITCODE
    $capRaw = & $ToolExe capture --title Rustamp --out $PngPath 2>&1 | Out-String
    $capRaw | Set-Content -Path (Join-Path $OutDir 'capture.log') -Encoding UTF8
    $capExit = $LASTEXITCODE
    Say 'capture exit' $capExit
    if ($capExit -ne 0) { Finish-Failure 'capture' ("the capture tool failed with exit {0}; see capture.log" -f $capExit) }
    $capturedTitle = ''
    $capturedHwnd = ''
    $capturedSize = ''
    if ($capRaw -match 'captured hwnd=0x([0-9A-Fa-f]+) class:=(".*?"|\S+) title:=(".*?"|\S+) -> \S+ \((\d+x\d+)\)') {
        $capturedHwnd = $Matches[1]; $capturedTitle = $Matches[3]; $capturedSize = $Matches[4]
    }
    Say 'captured' ($capRaw -split "`r?`n" | Where-Object { $_ -match 'captured hwnd' } | Select-Object -First 1)
    if (-not (Test-Path $PngPath)) { Finish-Failure 'capture' 'the capture tool reported success but wrote no PNG' }
    $pngBytes = (Get-Item $PngPath).Length
    Say 'png bytes' $pngBytes
    $sig = [System.IO.File]::ReadAllBytes($PngPath)[0..3]
    $isPng = ($sig[0] -eq 0x89 -and $sig[1] -eq 0x50 -and $sig[2] -eq 0x4E -and $sig[3] -eq 0x47)
    if (-not $isPng -or $pngBytes -lt 512) { Finish-Failure 'capture' ("the capture is not a plausible PNG (signature ok: {0}, bytes: {1})" -f $isPng, $pngBytes) }

    # --- stop the player, read its testimony ----------------------------------
    $upSeconds = [math]::Round(((Get-Date) - $startedAt).TotalSeconds, 1)
    Say 'player up (s)' $upSeconds
    $player.Kill()
    if (-not $player.WaitForExit(10000)) { Finish-Failure 'stop' 'the player did not die within 10 s of Kill; refusing to leave it running' }
    Say 'player exit code' $player.ExitCode
    $player.StandardOutput.ReadToEnd() | Set-Content -Path $StdoutLog -Encoding UTF8
    $player.StandardError.ReadToEnd() | Set-Content -Path $StderrLog -Encoding UTF8
    $stdoutText = Get-Content -Path $StdoutLog -Raw
    $silentMarker = $stdoutText -match 'output null \(windowed; silent, no audio device\)'
    Say 'silent-path marker' $silentMarker
    if (-not $silentMarker) { Finish-Failure 'silent-path' 'the player never printed `output null (windowed; silent, no audio device)`; the run did not take the silent path' }

    # The host must not outlive its player (parent-death termination): a
    # lingering child is a finding, recorded but not a smoke failure.
    Start-Sleep -Seconds 2
    $lingering = @(Get-CimInstance Win32_Process -Filter ("ParentProcessId = " + $player.Id) -ErrorAction SilentlyContinue |
        Where-Object { $_.Name -ieq 'rustamp-plughost32.exe' })
    $hostReaped = ($lingering.Count -eq 0)
    Say 'host reaped with player' $hostReaped

    # --- proof, manifest, sentinel --------------------------------------------
    $proof = [pscustomobject]@{
        ok            = $true
        session       = $me
        user          = $env:USERNAME
        computer      = $env:COMPUTERNAME
        screen        = $screen.ToString()
        playerPid     = $player.Id
        upSeconds     = $upSeconds
        windowHwnd    = ("0x{0:X}" -f $mainHwnd)
        windowTitle   = 'Rustamp'
        windowClient  = $mainClient
        windowRect    = $mainRect
        hostChildPid  = $hostPid
        hostReaped    = $hostReaped
        captureFile   = (Split-Path $PngPath -Leaf)
        captureBytes  = $pngBytes
        captureHwnd   = $capturedHwnd
        captureTitle  = $capturedTitle
        captureSize   = $capturedSize
        stdoutMarker  = [bool]$silentMarker
    }
    $proof | ConvertTo-Json -Depth 6 | Set-Content -Path $ProofPath -Encoding UTF8
    $log | Set-Content -Path $Transcript -Encoding UTF8
    $hashes = [ordered]@{}
    foreach ($f in Get-ChildItem -Path $OutDir -File -Recurse | Sort-Object FullName) {
        $rel = $f.FullName.Substring($OutDir.Length + 1)
        if ($rel -ieq 'hashes.json') { continue }
        $hashes[$rel] = (Get-FileHash -Path $f.FullName -Algorithm SHA256).Hash
    }
    $hashes | ConvertTo-Json | Set-Content -Path (Join-Path $OutDir 'hashes.json') -Encoding UTF8
    @{ ok = $true; failed = 0 } | ConvertTo-Json | Set-Content -Path $DonePath -Encoding UTF8
    exit 0
} catch {
    $sess = $null
    try { $sess = (Get-Process -Id $PID -ErrorAction Stop).SessionId } catch { $sess = $null }
    $log | Set-Content -Path $Transcript -Encoding UTF8
    [pscustomobject]@{
        ok      = $false
        session = $sess
        failed  = 'inner-exception'
        detail  = $_.Exception.Message
    } | ConvertTo-Json -Depth 6 | Set-Content -Path $ProofPath -Encoding UTF8
    @{ ok = $false; failed = 'inner-exception' } | ConvertTo-Json | Set-Content -Path $DonePath -Encoding UTF8
    exit 3
}
'@
$inner = $inner.Replace('__PLAYER__', $playerExe).Replace('__FIXTURE__', $fixture).Replace('__TOOL__', $toolExe).Replace('__OUTDIR__', $OutDir)
$inner | Set-Content -Path $InnerPath -Encoding UTF8
Show 'inner payload' $InnerPath

# --- 4. register the one-shot task in the interactive session ------------------
# Precedent: register-broker-task.ps1 (action + principal + trigger), moved
# inside the guest and pointed at the console session instead of S4U. Same
# shape as run-gui-session.ps1: LogonType Interactive (this user, when logged
# on: the console desktop) instead of S4U (session 0), Limited instead of
# Highest (the smoke needs no elevation; an elevated interactive task would
# add a UAC question the run cannot answer), and a one-shot trigger.
Unregister-ScheduledTask -TaskName $TaskName -Confirm:$false -ErrorAction SilentlyContinue
$action = New-ScheduledTaskAction -Execute 'powershell.exe' `
    -Argument ('-NoProfile -NonInteractive -ExecutionPolicy Bypass -File "{0}"' -f $InnerPath)
$principal = New-ScheduledTaskPrincipal -UserId ("{0}\{1}" -f $interactiveDomain, $interactiveUser) `
    -LogonType Interactive -RunLevel Limited
$trigger = New-ScheduledTaskTrigger -Once -At (Get-Date).AddMinutes(1)
Register-ScheduledTask -TaskName $TaskName -Action $action -Principal $principal `
    -Trigger $trigger -Force | Out-Null
Show 'smoke task' ("registered '{0}' as {1}\{2} (Interactive, session {3}, one-shot)" -f $TaskName, $interactiveDomain, $interactiveUser, $interactiveSession)

# --- 5. run it now and wait for the sentinel -----------------------------------
Start-ScheduledTask -TaskName $TaskName | Out-Null
$deadline = (Get-Date).AddMinutes(10)
$done = $null
while ((Get-Date) -lt $deadline) {
    Start-Sleep -Seconds 5
    if (Test-Path $DonePath) {
        try { $done = Get-Content -Path $DonePath -Raw | ConvertFrom-Json } catch { continue }
        break
    }
}
Unregister-ScheduledTask -TaskName $TaskName -Confirm:$false -ErrorAction SilentlyContinue
if (-not $done) { throw 'STEP guest FAILED: the smoke task produced no done sentinel within 10 minutes; its outputs (if any) are under C:\rustamp\out\smoke\' }
if (-not (Test-Path $ProofPath)) { throw 'STEP guest FAILED: the smoke task finished without a proof file; refusing instead of reporting a pass' }
$proof = Get-Content -Path $ProofPath -Raw | ConvertFrom-Json

# --- 6. trust checks: session 0 must fail loudly ---------------------------------
# A proof with no session field is a diagnostic failure of its own (it used to
# format as `ran in session , ...`, masking the real step failure): refuse and
# name the failed step instead of reporting an empty session.
if ($null -eq $proof.session -or [string]::IsNullOrWhiteSpace([string]$proof.session)) { throw ("STEP guest FAILED: the smoke task produced a proof with no session field (step '{0}': {1}); the session cannot be proven and the run is refused" -f $proof.failed, $proof.detail) }
if ($proof.session -eq 0) { throw 'STEP guest FAILED: the smoke task ran in session 0, which has no desktop; this run proves nothing and is reported as failed' }
if ($proof.session -ne $interactiveSession) {
    throw ("STEP guest FAILED: the smoke task ran in session {0}, not the interactive session {1}; refusing" -f $proof.session, $interactiveSession)
}
if (-not $proof.ok) { throw ("STEP guest FAILED: step '{0}' failed in the guest ({1}); transcript and logs are under C:\rustamp\out\smoke\ (copy out with the broker copyout op)" -f $proof.failed, $proof.detail) }

# The outer half's own trail joins the transcript (session discovery is the
# number-one historical failure point, and it happens before the inner
# payload writes anything).
$guestTranscript = Join-Path $OutDir 'guest-transcript.txt'
if (Test-Path $guestTranscript) {
    '' | Add-Content -Path $guestTranscript -Encoding UTF8
    $transcript | Add-Content -Path $guestTranscript -Encoding UTF8
}

# Single-object result contract: this object is the ONLY stdout of this
# script (progress above lives in $transcript, which the host prints from
# the copied-out guest-transcript.txt).
[pscustomobject]@{
    host         = $proof.computer
    user         = $proof.user
    session      = $proof.session
    screen       = $proof.screen
    playerPid    = $proof.playerPid
    upSeconds    = $proof.upSeconds
    windowHwnd   = $proof.windowHwnd
    windowTitle  = $proof.windowTitle
    windowClient = $proof.windowClient
    windowRect   = $proof.windowRect
    hostChildPid = $proof.hostChildPid
    hostReaped   = $proof.hostReaped
    captureFile  = $proof.captureFile
    captureBytes = $proof.captureBytes
    captureHwnd  = $proof.captureHwnd
    captureTitle = $proof.captureTitle
    captureSize  = $proof.captureSize
    stdoutMarker = $proof.stdoutMarker
}
