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

# --- 3. one job through the harness entry point --------------------------------
# Same bootstrap command the sandbox LogonCommand ran, with the staged paths
# remapped from the sandbox mappings to the VM staging root. The harness
# itself performs the 1:1 gate (client rect 275x116, capture 275x116, 0-px
# landmark plus SHA-256) and refuses on mismatch; this script does not
# second-guess it, it records the outcome.
if (-not (Test-Path $CaptureRoot)) { throw "nothing staged at $CaptureRoot" }
$bootstrap = Join-Path $CaptureRoot 'tools\vm-capture\sandbox-bootstrap.ps1'
$jobFile = Join-Path $CaptureRoot ("tools\vm-capture\jobs\" + $Job + ".json")
if (-not (Test-Path $bootstrap)) { throw "harness entry point not staged: $bootstrap" }
if (-not (Test-Path $jobFile)) { throw "job file not staged: $jobFile" }
if (Test-Path $OutDir) { Remove-Item -Recurse -Force $OutDir }
$null = New-Item -ItemType Directory -Force -Path $OutDir
Show 'harness exit' 'running...'
& powershell.exe -NoProfile -ExecutionPolicy Bypass -File $bootstrap -Job $jobFile -NoLogoff
$harnessExit = $LASTEXITCODE
Show 'harness exit' $harnessExit
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
