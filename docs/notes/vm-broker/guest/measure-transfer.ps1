# Measure the reference player's transfer display texts and window geometry.
#
# Allow-listed guest script. Called by the broker:
#   -Op guest -Script measure-transfer.ps1 -GuestArgs <mode>
#
# Modes: normal | refused | cut
#   normal: trickle body over ~25 s, full file
#   refused: TCP reject (no listener on the target port)
#   cut: send headers + ~10% body, then close
#
# The script follows the run-presented.ps1 pattern:
# - The HTTP server stays in session 0 (a listener needs no desktop)
# - The player launch and the ENTIRE observation loop run inside the
#   interactive session via a one-shot scheduled task (Interactive logon)
# - Session 0 only polls for a done sentinel and refuses loudly if the
#   task landed in session 0
# - Fixed run-directory name (transfer-<mode>) and summary.json/hash outputs
#   the host runner already expects

param([string]$Mode = 'normal')

$ErrorActionPreference = 'Stop'

if ($Mode -notin @('normal', 'refused', 'cut')) {
    throw "mode must be 'normal', 'refused', or 'cut', got '$Mode'"
}

$OutRoot = 'C:\rustamp\out'
$JobName = "transfer-$Mode"
$script:RunStamp = (Get-Date -Format 'yyyyMMdd-HHmmss')
$OutDir = Join-Path $OutRoot $JobName
$null = New-Item -ItemType Directory -Force -Path $OutDir

$transcript = New-Object System.Collections.Generic.List[string]
function Log($msg) { $transcript.Add(("{0:HH:mm:ss.fff} {1}" -f (Get-Date), $msg)); Write-Host $msg }

$logPath = Join-Path $OutDir 'guest-transcript.txt'
$observationsPath = Join-Path $OutDir 'observations.jsonl'
$serverLogPath = Join-Path $OutDir 'server-log.txt'
$innerDonePath = Join-Path $OutDir 'inner-done.json'

# -----------------------------------------------------------------------------
# 0. Find the interactive session (same logic as run-presented.ps1)
# -----------------------------------------------------------------------------
$interactiveSession = $null
$interactiveUser = $null
$interactiveDomain = $null
$explorer = Get-Process -Name explorer -ErrorAction SilentlyContinue | Select-Object -First 1
if ($explorer) {
    $interactiveSession = $explorer.SessionId
    $owner = (Get-CimInstance Win32_Process -Filter ("ProcessId = " + $explorer.Id) | Invoke-CimMethod -MethodName GetOwner)
    $interactiveUser = $owner.User
    $interactiveDomain = $owner.Domain
    Log "interactive session {0} via explorer (user {1}\{2})" -f $interactiveSession, $interactiveDomain, $interactiveUser
} else {
    Log 'no explorer; falling back to `query session`'
    foreach ($line in (query session 2>$null)) {
        if ($line -match '^\s*>?\s*console\s+(\S+)\s+(\d+)\s+Active') {
            $interactiveUser = $Matches[1]
            $interactiveSession = [int]$Matches[2]
            $interactiveDomain = $env:COMPUTERNAME
            break
        }
    }
}
if ($null -eq $interactiveSession) { throw 'no interactive session found: explorer is not running and `query session` shows no Active console; the player has no desktop to appear on' }
if ($interactiveSession -eq 0) { throw 'the "interactive" session resolved to session 0, which has no desktop; refusing instead of reporting a pass' }
if ([string]::IsNullOrWhiteSpace($interactiveUser)) { throw 'interactive session found but its user could not be resolved; refusing' }

# -----------------------------------------------------------------------------
# 1. Slow HTTP server (HttpListener) - runs in session 0 (this process)
# -----------------------------------------------------------------------------
$serverPort = 18080
$serverUrl = "http://127.0.0.1:$serverPort/"
$audioPath = 'C:\rustamp\testmedia\jingle-rockdown.mp3'
$audioBytes = if (Test-Path $audioPath) { [IO.File]::ReadAllBytes($audioPath) } else {
    Log "No test media at $audioPath, generating a small WAV"
    $sampleRate = 8000
    $durationSec = 1
    $samples = $sampleRate * $durationSec
    $wave = New-Object byte[] (44 + $samples)
    [Text.Encoding]::ASCII.GetBytes('RIFF').CopyTo($wave, 0)
    [BitConverter]::GetBytes(36 + $samples).CopyTo($wave, 4)
    [Text.Encoding]::ASCII.GetBytes('WAVE').CopyTo($wave, 8)
    [Text.Encoding]::ASCII.GetBytes('fmt ').CopyTo($wave, 12)
    [BitConverter]::GetBytes(16).CopyTo($wave, 16)
    [BitConverter]::GetBytes([System.UInt16]1).CopyTo($wave, 20)
    [BitConverter]::GetBytes([System.UInt16]1).CopyTo($wave, 22)
    [BitConverter]::GetBytes($sampleRate).CopyTo($wave, 24)
    [BitConverter]::GetBytes($sampleRate).CopyTo($wave, 28)
    [BitConverter]::GetBytes([System.UInt16]1).CopyTo($wave, 32)
    [BitConverter]::GetBytes([System.UInt16]8).CopyTo($wave, 34)
    [Text.Encoding]::ASCII.GetBytes('data').CopyTo($wave, 36)
    [BitConverter]::GetBytes($samples).CopyTo($wave, 40)
    for ($i = 0; $i -lt $samples; $i++) {
        $val = [math]::Sin(2 * [math]::PI * 440 * $i / $sampleRate)
        $wave[44 + $i] = [byte](128 + [math]::Round($val * 127))
    }
    $wave
}
$contentLength = $audioBytes.Length
Log "Audio file: $contentLength bytes"

# Write the server script to a temporary file
$serverScriptPath = Join-Path $OutDir 'server.ps1'
$serverScriptContent = @'
param($Mode, $Port, $AudioBytesPath, $ContentLength, $LogPath)

$audioBytes = [IO.File]::ReadAllBytes($AudioBytesPath)

$listener = New-Object Net.HttpListener
$listener.Prefixes.Add("http://127.0.0.1:$Port/") | Out-Null
$listener.Start()
"Listener started on port $Port" | Out-File -FilePath $LogPath -Encoding UTF8 -Append

try {
    while ($true) {
        $ctx = $listener.GetContext()
        $req = $ctx.Request
        $resp = $ctx.Response
        $reqLine = "$($req.HttpMethod) $($req.RawUrl) HTTP/$($req.ProtocolVersion)"
        "$(Get-Date -Format 'HH:mm:ss.fff') REQUEST: $reqLine" | Out-File -FilePath $LogPath -Encoding UTF8 -Append
        foreach ($headerKey in $req.Headers.AllKeys) {
            ("$(Get-Date -Format 'HH:mm:ss.fff')   Header: {0}: {1}" -f $headerKey, $req.Headers[$headerKey]) | Out-File -FilePath $LogPath -Encoding UTF8 -Append
        }

        if ($Mode -eq 'refused') {
            $resp.StatusCode = 500
            $resp.Close()
            continue
        }

        # Send headers
        try {
            $resp.ProtocolVersion = New-Object Version(1,0)
            $resp.StatusCode = 200
            $resp.ContentType = 'audio/mpeg'
            $resp.ContentLength64 = $ContentLength
            $resp.Headers.Add('Accept-Ranges', 'bytes')
            # Headers are sent automatically on first write to OutputStream in .NET Framework
            # No SendHeaders() method exists
        }
        catch {
            "$(Get-Date -Format 'HH:mm:ss.fff') SET HEADERS ERROR: $($_.Exception.Message)" | Out-File -FilePath $LogPath -Encoding UTF8 -Append
            $resp.Close()
            continue
        }

        "$(Get-Date -Format 'HH:mm:ss.fff') HEADERS SET" | Out-File -FilePath $LogPath -Encoding UTF8 -Append

        $total = $ContentLength
        $sent = 0
        $chunkSize = 1024  # 1 KB per chunk
        $delayMs = 50      # ~20 KB/s -> ~25 s for 500 KB file

        if ($Mode -eq 'cut') {
            $cutAt = [math]::Max(1, [math]::Floor($total * 0.1))
        } else {
            $cutAt = $total
        }

        try {
            while ($sent -lt $cutAt) {
                $thisChunk = [math]::Min($chunkSize, $cutAt - $sent)
                $resp.OutputStream.Write($audioBytes, $sent, $thisChunk)
                $resp.OutputStream.Flush()
                $sent += $thisChunk
                if ($sent % 10240 -eq 0 -or $sent -eq $cutAt) {
                    "$(Get-Date -Format 'HH:mm:ss.fff') SENT $sent / $cutAt bytes" | Out-File -FilePath $LogPath -Encoding UTF8 -Append
                }
                Start-Sleep -Milliseconds $delayMs
            }
            "$(Get-Date -Format 'HH:mm:ss.fff') BODY COMPLETE (sent $sent bytes)" | Out-File -FilePath $LogPath -Encoding UTF8 -Append
        }
        catch {
            "$(Get-Date -Format 'HH:mm:ss.fff') WRITE ERROR: $($_.Exception.Message)" | Out-File -FilePath $LogPath -Encoding UTF8 -Append
        }
        finally {
            $resp.OutputStream.Close()
            $resp.Close()
        }
    }
}
catch {
    "$(Get-Date -Format 'HH:mm:ss.fff') LISTENER LOOP ERROR: $($_.Exception.Message)" | Out-File -FilePath $LogPath -Encoding UTF8 -Append
}
finally {
    $listener.Stop()
    "$(Get-Date -Format 'HH:mm:ss.fff') LISTENER STOPPED" | Out-File -FilePath $LogPath -Encoding UTF8 -Append
}
'@

$serverScriptContent | Set-Content -Path $serverScriptPath -Encoding UTF8

# Save audio bytes to a file for the server process
$audioBytesPath = Join-Path $OutDir 'audio.bin'
[IO.File]::WriteAllBytes($audioBytesPath, $audioBytes)

# Start server in a separate process (unless refused mode)
$serverProc = $null
if ($Mode -ne 'refused') {
    $serverArgs = "-NoProfile -ExecutionPolicy Bypass -File `"$serverScriptPath`" -Mode $Mode -Port $serverPort -AudioBytesPath `"$audioBytesPath`" -ContentLength $contentLength -LogPath `"$serverLogPath`""
    $serverProc = Start-Process -FilePath 'powershell.exe' -ArgumentList $serverArgs -WindowStyle Hidden -PassThru
    Log "Started HTTP server process ID $($serverProc.Id) on port $serverPort"
    Start-Sleep -Seconds 2  # Give it time to start
} else {
    Log "Refused mode: no server started (connection will be refused)"
}

# -----------------------------------------------------------------------------
# 2. Find reference Winamp 2.95 (staged by host)
# -----------------------------------------------------------------------------
$winampPath = @(
    'C:\rustamp\capture\winamp295\winamp.exe',
    'C:\Program Files\Winamp\winamp.exe',
    'C:\Program Files (x86)\Winamp\winamp.exe',
    'C:\Winamp\winamp.exe'
) | Where-Object { Test-Path $_ } | Select-Object -First 1

if (-not $winampPath) {
    throw "Winamp 2.95 not found in standard locations"
}
Log "Found Winamp at $winampPath"

# -----------------------------------------------------------------------------
# 3. Write the INNER payload script directly to a file (avoids here-string expansion issues)
# -----------------------------------------------------------------------------
$innerPath = Join-Path $OutDir 'inner.ps1'
$streamUrl = "$serverUrl" + "stream.mp3"

# Pre-compute values to avoid complex expressions in here-string
$serverProcId = if ($serverProc) { $serverProc.Id } else { 0 }
$innerTranscriptPath = Join-Path $OutDir 'inner-transcript.txt'

$innerScript = @"
`$ErrorActionPreference = 'Continue'
`$InnerDone = '$innerDonePath'
`$OutDir = '$OutDir'
`$WinampPath = '$winampPath'
`$StreamUrl = '$streamUrl'
`$ServerProcId = $serverProcId
`$Mode = '$Mode'
`$RunStamp = '$($script:RunStamp)'
`$ObservationsPath = '$observationsPath'
`$ServerLogPath = '$serverLogPath'
`$TranscriptPath = '$logPath'
`$InnerTranscriptPath = '$innerTranscriptPath'
`$AudioBytesPath = '$audioBytesPath'
`$ContentLength = $contentLength
`$ServerPort = $serverPort

`$transcript = New-Object System.Collections.Generic.List[string]
function Log(`$msg) { `$transcript.Add(('{0:HH:mm:ss.fff} {1}' -f (Get-Date), `$msg)); Write-Host `$msg }

# Fail loudly in session 0: the player cannot show a window there, and a
# pass from there would be a lie.
`$me = (Get-Process -Id `$PID).SessionId
`$failure = ''
`$observationCount = 0

if (`$me -eq 0) {
    `$failure = 'refusing: the measurement task landed in session 0, which has no desktop'
} else {
    Log "Running in interactive session `$me as `$env:USERNAME"

    # Win32 interop for window enumeration
    Add-Type @'
using System;
using System.Runtime.InteropServices;
using System.Collections.Generic;

public static class Win32 {
    public delegate bool EnumWindowsProc(IntPtr hWnd, IntPtr lParam);

    [DllImport("user32.dll", SetLastError = true)]
    public static extern bool EnumWindows(EnumWindowsProc lpEnumFunc, IntPtr lParam);

    [DllImport("user32.dll", SetLastError = true)]
    public static extern int GetWindowThreadProcessId(IntPtr hWnd, out int lpdwProcessId);

    [DllImport("user32.dll", SetLastError = true)]
    public static extern bool IsWindowVisible(IntPtr hWnd);

    [DllImport("user32.dll", SetLastError = true)]
    public static extern int GetClassName(IntPtr hWnd, System.Text.StringBuilder lpClassName, int nMaxCount);

    [DllImport("user32.dll", SetLastError = true)]
    public static extern int GetWindowText(IntPtr hWnd, System.Text.StringBuilder lpString, int nMaxCount);

    [DllImport("user32.dll", SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    public static extern bool GetClientRect(IntPtr hWnd, out RECT lpRect);

    [DllImport("user32.dll", SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    public static extern bool GetWindowRect(IntPtr hWnd, out RECT lpRect);

    [StructLayout(LayoutKind.Sequential)]
    public struct RECT {
        public int Left, Top, Right, Bottom;
    }
}
'@

    # Use a script-scope variable for the target PID to avoid closure issues with EnumWindows callback
    `$script:TargetPidForEnum = 0
    function Get-WinampWindows {
        param(`$TargetPid)
        `$script:TargetPidForEnum = `$TargetPid
        `$windows = New-Object System.Collections.Generic.List[pscustomobject]
        `$callback = {
            param(`$hWnd, `$lParam)
            `$pidOut = 0
            [Win32]::GetWindowThreadProcessId(`$hWnd, [ref]`$pidOut) | Out-Null
            if (`$pidOut -eq `$script:TargetPidForEnum) {
                `$visible = [Win32]::IsWindowVisible(`$hWnd)
                if (`$visible) {
                    `$classSb = New-Object System.Text.StringBuilder(256)
                    [Win32]::GetClassName(`$hWnd, `$classSb, `$classSb.Capacity) | Out-Null
                    `$titleSb = New-Object System.Text.StringBuilder(256)
                    [Win32]::GetWindowText(`$hWnd, `$titleSb, `$titleSb.Capacity) | Out-Null
                    `$clientRect = New-Object Win32+RECT
                    `$winRect = New-Object Win32+RECT
                    `$hasClient = [Win32]::GetClientRect(`$hWnd, [ref]`$clientRect)
                    `$hasWin = [Win32]::GetWindowRect(`$hWnd, [ref]`$winRect)
                    `$windows.Add([pscustomobject]@{
                        Hwnd       = '0x{0:X}' -f [int64]`$hWnd
                        Class      = `$classSb.ToString()
                        Title      = `$titleSb.ToString()
                        ClientRect = if (`$hasClient) { @{ Left=`$clientRect.Left; Top=`$clientRect.Top; Right=`$clientRect.Right; Bottom=`$clientRect.Bottom; Width=`$clientRect.Right-`$clientRect.Left; Height=`$clientRect.Bottom-`$clientRect.Top } } else { `$null }
                        WindowRect = if (`$hasWin) { @{ Left=`$winRect.Left; Top=`$winRect.Top; Right=`$winRect.Right; Bottom=`$winRect.Bottom; Width=`$winRect.Right-`$winRect.Left; Height=`$winRect.Bottom-`$winRect.Top } } else { `$null }
                    })
                }
            }
            return `$true
        }
        [Win32]::EnumWindows(`$callback, [IntPtr]::Zero) | Out-Null
        return `$windows
    }

    trap {
        `$message = (`$_ | Out-String).Trim()
        Log "TRAP: `$message"
        `$record = [ordered]@{
            session      = `$me
            user         = `$env:USERNAME
            computer     = `$env:COMPUTERNAME
            failure      = `$message
            observations = `$observationCount
        }
        `$record | ConvertTo-Json -Depth 5 | Set-Content -Path `$InnerDone -Encoding UTF8
        `$transcript | Set-Content -Path `$InnerTranscriptPath -Encoding UTF8
        exit 1
    }

    # Launch Winamp with the streaming URL, using /NOREG to skip setup wizard
    Log "Launching Winamp with URL: `$StreamUrl"
    `$proc = Start-Process -FilePath `$WinampPath -ArgumentList "/NOREG", `$StreamUrl -PassThru -WorkingDirectory (Split-Path `$WinampPath)
    `$winampPid = `$proc.Id
    Log "Winamp PID: `$winampPid"

    # Wait longer for Winamp to initialize and create windows
    Start-Sleep -Seconds 5

    # Use List to avoid scriptblock variable capture issues with EnumWindows callback
    `$allWindows = New-Object System.Collections.Generic.List[pscustomobject]
    `$callbackAll = {
        param(`$hWnd, `$lParam)
        `$pidOut = 0
        [Win32]::GetWindowThreadProcessId(`$hWnd, [ref]`$pidOut) | Out-Null
        if (`$pidOut -eq `$winampPid) {
            `$visible = [Win32]::IsWindowVisible(`$hWnd)
            `$classSb = New-Object System.Text.StringBuilder(256)
            [Win32]::GetClassName(`$hWnd, `$classSb, `$classSb.Capacity) | Out-Null
            `$titleSb = New-Object System.Text.StringBuilder(256)
            [Win32]::GetWindowText(`$hWnd, `$titleSb, `$titleSb.Capacity) | Out-Null
            `$allWindows.Add([pscustomobject]@{
                Hwnd       = '0x{0:X}' -f [int64]`$hWnd
                Class      = `$classSb.ToString()
                Title      = `$titleSb.ToString()
                Visible    = `$visible
            })
        }
        return `$true
    }
    [Win32]::EnumWindows(`$callbackAll, [IntPtr]::Zero) | Out-Null
    Log "ALL Winamp windows after 5s: `$(`$allWindows.Count)"
    `$allWindows | ForEach-Object { Log "  HWND=`$(`$_.Hwnd) Class=`$(`$_.Class) Title=`$(`$_.Title) Visible=`$(`$_.Visible)" }

    # Also enumerate ALL windows on desktop to see if EnumWindows works at all
    `$allDesktopWindows = New-Object System.Collections.Generic.List[pscustomobject]
    `$callbackDesktop = {
        param(`$hWnd, `$lParam)
        `$pidOut = 0
        [Win32]::GetWindowThreadProcessId(`$hWnd, [ref]`$pidOut) | Out-Null
        `$visible = [Win32]::IsWindowVisible(`$hWnd)
        `$classSb = New-Object System.Text.StringBuilder(256)
        [Win32]::GetClassName(`$hWnd, `$classSb, `$classSb.Capacity) | Out-Null
        `$titleSb = New-Object System.Text.StringBuilder(256)
        [Win32]::GetWindowText(`$hWnd, `$titleSb, `$titleSb.Capacity) | Out-Null
        `$allDesktopWindows.Add([pscustomobject]@{
            Hwnd       = '0x{0:X}' -f [int64]`$hWnd
            Class      = `$classSb.ToString()
            Title      = `$titleSb.ToString()
            Visible    = `$visible
            Pid        = `$pidOut
        })
        return `$true
    }
    [Win32]::EnumWindows(`$callbackDesktop, [IntPtr]::Zero) | Out-Null
    Log "Total desktop windows: `$(`$allDesktopWindows.Count)"
    `$winampDesktop = `$allDesktopWindows | Where-Object { `$_.Pid -eq `$winampPid }
    Log "Desktop windows for Winamp PID: `$(`$winampDesktop.Count)"
    `$winampDesktop | ForEach-Object { Log "  HWND=`$(`$_.Hwnd) Class=`$(`$_.Class) Title=`$(`$_.Title) Visible=`$(`$_.Visible) Pid=`$(`$_.Pid)" }

    # Also check MainWindowHandle from process
    `$procCheck = Get-Process -Id `$winampPid -ErrorAction SilentlyContinue
    if (`$procCheck) {
        Log "Winamp process alive: HasExited=`$(`$procCheck.HasExited), MainWindowTitle='`$(`$procCheck.MainWindowTitle)', MainWindowHandle=0x{0:X}" -f `$procCheck.MainWindowHandle
        if (`$procCheck.MainWindowHandle -ne 0) {
            `$hwnd = `$procCheck.MainWindowHandle
            `$visible = [Win32]::IsWindowVisible(`$hwnd)
            `$classSb = New-Object System.Text.StringBuilder(256)
            [Win32]::GetClassName(`$hwnd, `$classSb, `$classSb.Capacity) | Out-Null
            `$titleSb = New-Object System.Text.StringBuilder(256)
            [Win32]::GetWindowText(`$hwnd, `$titleSb, `$titleSb.Capacity) | Out-Null
            Log "MainWindowHandle details: Class=`$(`$classSb.ToString()) Title=`$(`$titleSb.ToString()) Visible=`$visible"
        }
    } else {
        Log "Winamp process NOT found!"
    }

    # Capture loop
    `$maxDurationSec = 40
    `$startTime = Get-Date
    `$deadline = `$startTime.AddSeconds(`$maxDurationSec)

    Log "Starting capture loop (max `$maxDurationSec seconds)"

    while ((Get-Date) -lt `$deadline) {
        `$loopStart = Get-Date
        `$windows = Get-WinampWindows -TargetPid `$winampPid

        `$obs = [pscustomobject]@{
            Timestamp   = `$loopStart.ToString('o')
            ElapsedSec  = [math]::Round((`$loopStart - `$startTime).TotalSeconds, 3)
            WindowCount = `$windows.Count
            Windows     = `$windows
        }

        `$obs | ConvertTo-Json -Depth 5 | Out-File -FilePath `$ObservationsPath -Encoding UTF8 -Append
        `$observationCount++

        `$winDesc = (`$windows | ForEach-Object { '[{0}] ''{1}'' {2}x{3}' -f `$_.Class, `$_.Title, `$_.ClientRect.Width, `$_.ClientRect.Height }) -join ' | '
        Log "  t=`$([math]::Round((`$loopStart - `$startTime).TotalSeconds,1))s windows=`$(`$windows.Count) `$winDesc"

        # Check if process exited
        `$proc = Get-Process -Id `$winampPid -ErrorAction SilentlyContinue
        if (-not `$proc -or `$proc.HasExited) {
            Log "Winamp process exited"
            break
        }

        # Sleep to maintain ~1 Hz
        `$elapsed = (Get-Date) - `$loopStart
        `$sleepMs = [int][math]::Max(100, 1000 - `$elapsed.TotalMilliseconds)
        Start-Sleep -Milliseconds `$sleepMs
    }
}

# Write final summary/record
`$record = [ordered]@{
    session       = `$me
    user          = `$env:USERNAME
    computer      = `$env:COMPUTERNAME
    failure       = `$failure
    observations  = `$observationCount
    winampPid     = if (`$winampPid) { `$winampPid } else { `$null }
    startTime     = if (`$startTime) { `$startTime.ToString('o') } else { `$null }
    endTime       = (Get-Date).ToString('o')
    durationSec   = if (`$startTime) { [math]::Round(((Get-Date) - `$startTime).TotalSeconds, 3) } else { 0 }
}
`$record | ConvertTo-Json -Depth 5 | Set-Content -Path `$InnerDone -Encoding UTF8
`$transcript | Set-Content -Path `$InnerTranscriptPath -Encoding UTF8
Log "Inner payload done. Observations: `$observationCount. Session: `$me. Failure: `$failure"
exit 0
"@

$innerScript | Set-Content -Path $innerPath -Encoding UTF8

# -----------------------------------------------------------------------------
# 4. Register and start the one-shot scheduled task in the interactive session
# -----------------------------------------------------------------------------
$taskName = 'RustampMeasureTransfer'
Unregister-ScheduledTask -TaskName $taskName -Confirm:$false -ErrorAction SilentlyContinue

$action = New-ScheduledTaskAction -Execute 'powershell.exe' `
    -Argument ("-NoProfile -NonInteractive -ExecutionPolicy Bypass -File `"$innerPath`"")
$principal = New-ScheduledTaskPrincipal -UserId ("{0}\{1}" -f $interactiveDomain, $interactiveUser) `
    -LogonType Interactive -RunLevel Limited
$trigger = New-ScheduledTaskTrigger -Once -At (Get-Date).AddMinutes(1)
Register-ScheduledTask -TaskName $taskName -Action $action -Principal $principal `
    -Trigger $trigger -Force | Out-Null
Log "Registered task '$taskName' as $interactiveDomain\$interactiveUser (Interactive, session $interactiveSession)"
Start-ScheduledTask -TaskName $taskName

# -----------------------------------------------------------------------------
# 5. Session 0 waits for the done sentinel
# -----------------------------------------------------------------------------
$deadline = (Get-Date).AddMinutes(6)
$innerDoneObj = $null
while ((Get-Date) -lt $deadline) {
    Start-Sleep -Seconds 5
    if (Test-Path $innerDonePath) {
        try { $innerDoneObj = Get-Content -Path $innerDonePath -Raw | ConvertFrom-Json } catch { continue }
        break
    }
}
Unregister-ScheduledTask -TaskName $taskName -Confirm:$false -ErrorAction SilentlyContinue

if (-not $innerDoneObj) { throw 'the measurement task produced no done sentinel within 6 minutes; refusing instead of reporting a pass' }
if ($innerDoneObj.session -eq 0) { throw ("the measurement task ran in session 0 ({0}); refusing" -f $innerDoneObj.failure) }
if ($innerDoneObj.session -ne $interactiveSession) { throw ("the measurement task ran in session {0}, not the interactive session {1}; refusing" -f $innerDoneObj.session, $interactiveSession) }

Log "Inner task ran in session $($innerDoneObj.session) as $($innerDoneObj.user) - OK"
Log "Observations recorded: $($innerDoneObj.observations)"

# -----------------------------------------------------------------------------
# 6. Cleanup server process
# -----------------------------------------------------------------------------
if ($serverProc) {
    Log "Stopping HTTP server process"
    Stop-Process -Id $serverProc.Id -Force -ErrorAction SilentlyContinue
    Start-Sleep -Seconds 1
}

# -----------------------------------------------------------------------------
# 7. Write summary (matching the host runner's expected format)
# -----------------------------------------------------------------------------
$summary = [pscustomobject]@{
    Job        = $JobName
    Stamp      = $script:RunStamp
    Mode       = $Mode
    WinampPid  = $innerDoneObj.winampPid
    StartTime  = $innerDoneObj.startTime
    EndTime    = $innerDoneObj.endTime
    DurationSec = $innerDoneObj.durationSec
    Observations = $innerDoneObj.observations
    OutDir     = $OutDir
}
$summary | ConvertTo-Json -Depth 3 | Set-Content -Path (Join-Path $OutDir 'summary.json') -Encoding UTF8
$transcript | Set-Content -Path $logPath -Encoding UTF8

Log "Done. Observations: $($innerDoneObj.observations). Output: $OutDir"

# Emit exactly one summary object on stdout
$summary