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
# The script:
# 1. Starts a slow HttpListener on 127.0.0.1 inside the guest
# 2. Launches the reference Winamp 2.95 with the URL as argument
# 3. Captures every visible top-level window of the player process once per second
# 4. Reports each capture's class, title, client size, and window rect
# 5. Writes a JSONL file with all observations

param([string]$Mode = 'normal')

$ErrorActionPreference = 'Stop'

if ($Mode -notin @('normal', 'refused', 'cut')) {
    throw "mode must be 'normal', 'refused', or 'cut', got '$Mode'"
}

$OutRoot = 'C:\rustamp\out'
$JobName = "transfer-$Mode-$(Get-Date -Format 'yyyyMMdd-HHmmss')"
$OutDir = Join-Path $OutRoot $JobName
$null = New-Item -ItemType Directory -Force -Path $OutDir

$transcript = New-Object System.Collections.Generic.List[string]
function Log($msg) { $transcript.Add(("{0:HH:mm:ss.fff} {1}" -f (Get-Date), $msg)); Write-Host $msg }

$logPath = Join-Path $OutDir 'guest-transcript.txt'
$observationsPath = Join-Path $OutDir 'observations.jsonl'
$serverLogPath = Join-Path $OutDir 'server-log.txt'

# -----------------------------------------------------------------------------
# 1. Slow HTTP server (HttpListener) - runs in a separate PowerShell process
# -----------------------------------------------------------------------------
$serverPort = 18080
$serverUrl = "http://127.0.0.1:$serverPort/"
$audioPath = 'C:\rustamp\testmedia\jingle-rockdown.mp3'
$audioBytes = if (Test-Path $audioPath) { [IO.File]::ReadAllBytes($audioPath) } else {
    Log "No test media at $audioPath, generating a small WAV"
    # Generate a tiny valid WAV file (1 second, 440 Hz, 8-bit mono, 8 kHz)
    $sampleRate = 8000
    $durationSec = 1
    $samples = $sampleRate * $durationSec
    $wave = New-Object byte[] (44 + $samples)
    # RIFF header
    [Text.Encoding]::ASCII.GetBytes('RIFF').CopyTo($wave, 0)
    [BitConverter]::GetBytes(36 + $samples).CopyTo($wave, 4)
    [Text.Encoding]::ASCII.GetBytes('WAVE').CopyTo($wave, 8)
    [Text.Encoding]::ASCII.GetBytes('fmt ').CopyTo($wave, 12)
    [BitConverter]::GetBytes(16).CopyTo($wave, 16)   # fmt chunk size
    [BitConverter]::GetBytes([System.UInt16]1).CopyTo($wave, 20)  # PCM
    [BitConverter]::GetBytes([System.UInt16]1).CopyTo($wave, 22)  # mono
    [BitConverter]::GetBytes($sampleRate).CopyTo($wave, 24)
    [BitConverter]::GetBytes($sampleRate).CopyTo($wave, 28)  # byte rate = sample rate * 1 * 1
    [BitConverter]::GetBytes([System.UInt16]1).CopyTo($wave, 32)  # block align
    [BitConverter]::GetBytes([System.UInt16]8).CopyTo($wave, 34)  # bits per sample
    [Text.Encoding]::ASCII.GetBytes('data').CopyTo($wave, 36)
    [BitConverter]::GetBytes($samples).CopyTo($wave, 40)
    # 440 Hz sine wave, 8-bit unsigned
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
        $resp.ProtocolVersion = New-Object Version(1,0)
        $resp.StatusCode = 200
        $resp.ContentType = 'audio/mpeg'
        $resp.ContentLength64 = $ContentLength
        $resp.Headers.Add('Accept-Ranges', 'bytes')
        $resp.SendHeaders()

        "$(Get-Date -Format 'HH:mm:ss.fff') HEADERS SENT" | Out-File -FilePath $LogPath -Encoding UTF8 -Append

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
# 2. Find reference Winamp 2.95
# -----------------------------------------------------------------------------
# The reference is STAGED (the host copies the player directory in), so the
# staged path comes first. The Program Files entries stay as a fallback for a
# guest that happens to have an installed copy.
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
# 3. Launch Winamp with the streaming URL
# -----------------------------------------------------------------------------
$streamUrl = "$serverUrl/stream.mp3"
Log "Launching Winamp with URL: $streamUrl"

$proc = Start-Process -FilePath $winampPath -ArgumentList $streamUrl -PassThru -WorkingDirectory (Split-Path $winampPath)
$winampPid = $proc.Id
Log "Winamp PID: $winampPid"

# -----------------------------------------------------------------------------
# 4. Capture loop - enumerate all visible top-level windows of the process
# -----------------------------------------------------------------------------
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

function Get-WinampWindows {
    param($TargetPid)
    $windows = @()
    $callback = {
        param($hWnd, $lParam)
        $pidOut = 0
        [Win32]::GetWindowThreadProcessId($hWnd, [ref]$pidOut) | Out-Null
        if ($pidOut -eq $TargetPid) {
            $visible = [Win32]::IsWindowVisible($hWnd)
            if ($visible) {
                $classSb = New-Object System.Text.StringBuilder(256)
                [Win32]::GetClassName($hWnd, $classSb, $classSb.Capacity) | Out-Null
                $titleSb = New-Object System.Text.StringBuilder(256)
                [Win32]::GetWindowText($hWnd, $titleSb, $titleSb.Capacity) | Out-Null
                $clientRect = New-Object Win32+RECT
                $winRect = New-Object Win32+RECT
                $hasClient = [Win32]::GetClientRect($hWnd, [ref]$clientRect)
                $hasWin = [Win32]::GetWindowRect($hWnd, [ref]$winRect)
                $windows += [pscustomobject]@{
                    Hwnd       = "0x{0:X}" -f [int64]$hWnd
                    Class      = $classSb.ToString()
                    Title      = $titleSb.ToString()
                    ClientRect = if ($hasClient) { @{ Left=$clientRect.Left; Top=$clientRect.Top; Right=$clientRect.Right; Bottom=$clientRect.Bottom; Width=$clientRect.Right-$clientRect.Left; Height=$clientRect.Bottom-$clientRect.Top } } else { $null }
                    WindowRect = if ($hasWin) { @{ Left=$winRect.Left; Top=$winRect.Top; Right=$winRect.Right; Bottom=$winRect.Bottom; Width=$winRect.Right-$winRect.Left; Height=$winRect.Bottom-$winRect.Top } } else { $null }
                }
            }
        }
        return $true
    }
    [Win32]::EnumWindows($callback, [IntPtr]::Zero) | Out-Null
    return $windows
}

# Capture loop
$maxDurationSec = 40
$startTime = Get-Date
$deadline = $startTime.AddSeconds($maxDurationSec)
$observationCount = 0

Log "Starting capture loop (max $maxDurationSec seconds)"

while ((Get-Date) -lt $deadline) {
    $loopStart = Get-Date
    $windows = Get-WinampWindows -TargetPid $winampPid

    $obs = [pscustomobject]@{
        Timestamp   = $loopStart.ToString('o')
        ElapsedSec  = [math]::Round(($loopStart - $startTime).TotalSeconds, 3)
        WindowCount = $windows.Count
        Windows     = $windows
    }

    $obs | ConvertTo-Json -Depth 5 | Out-File -FilePath $observationsPath -Encoding UTF8 -Append
    $observationCount++

    $winDesc = ($windows | ForEach-Object { "[{0}] '{1}' {2}x{3}" -f $_.Class, $_.Title, $_.ClientRect.Width, $_.ClientRect.Height }) -join ' | '
    Log "  t=$([math]::Round(($loopStart - $startTime).TotalSeconds,1))s windows=$($windows.Count) $winDesc"

    # Check if process exited
    $proc = Get-Process -Id $winampPid -ErrorAction SilentlyContinue
    if (-not $proc -or $proc.HasExited) {
        Log "Winamp process exited"
        break
    }

    # Sleep to maintain ~1 Hz
    $elapsed = (Get-Date) - $loopStart
    $sleepMs = [int][math]::Max(100, 1000 - $elapsed.TotalMilliseconds)
    Start-Sleep -Milliseconds $sleepMs
}

# -----------------------------------------------------------------------------
# 5. Cleanup
# -----------------------------------------------------------------------------
if ($serverProc) {
    Log "Stopping HTTP server process"
    Stop-Process -Id $serverProc.Id -Force -ErrorAction SilentlyContinue
    Start-Sleep -Seconds 1
}

# Get server log
if (Test-Path $serverLogPath) {
    $serverLog = Get-Content $serverLogPath -Raw
    Log "Server log captured ($($serverLog.Length) chars)"
}

# Write summary
$summary = [pscustomobject]@{
    Job        = $JobName
    Mode       = $Mode
    WinampPid  = $winampPid
    StartTime  = $startTime.ToString('o')
    EndTime    = (Get-Date).ToString('o')
    DurationSec = [math]::Round(((Get-Date) - $startTime).TotalSeconds, 3)
    Observations = $observationCount
    OutDir     = $OutDir
}
$summary | ConvertTo-Json -Depth 3 | Set-Content -Path (Join-Path $OutDir 'summary.json') -Encoding UTF8
$transcript | Set-Content -Path $logPath -Encoding UTF8

Log "Done. Observations: $observationCount. Output: $OutDir"

# Emit exactly one summary object on stdout
$summary