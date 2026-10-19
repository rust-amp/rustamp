# Run the Rustamp test binaries INSIDE the guest.  Allow-listed guest script.
#
# Shared payload for run-gui-session.ps1 (the default interactive path).
# Direct broker execution in session 0 is refused except for the deliberately
# named --desktop-free-residents:pcm_process / :dsp_process selections (W21).
# It expects the host to have compiled and staged C:\rustamp\code\Rustamp.
#
# The trick that makes "compile on the host, run in the guest" work without
# touching a single test: the tests locate the repository, the fixtures and the
# built binary through paths baked in at COMPILE time (the host's
# D:\code\Rustamp\...). `subst D: C:\rustamp` makes exactly those paths resolve
# inside the guest, so the same binaries run unchanged.
#
# A console test binary can still open a window or spawn a GUI helper. Running
# the suite in session 0 is therefore NOT safe evidence (owner, 2026-10-18).

param([string]$Filter = '')

$ErrorActionPreference = 'Continue'
$session = (Get-Process -Id $PID -ErrorAction Stop).SessionId
$desktopFree = $Filter -cmatch '^--desktop-free-residents:(pcm_process|dsp_process)$'
if ($desktopFree) {
    $Filter = '--ignored:' + $Matches[1]
} elseif ($session -eq 0 -or $Filter -like '--desktop-free*') {
    throw 'session-0 suite refused: use run-gui-session.ps1; only --desktop-free-residents:pcm_process or :dsp_process is allowed without a desktop'
}
$logDir = if (-not $desktopFree) { 'C:\rustamp\out\gui-session' } else { $null }

# 1. make the baked host paths resolve
if (-not (Test-Path 'D:\code\Rustamp')) {
    subst D: C:\rustamp
}
$repo = 'D:\code\Rustamp'
if (-not (Test-Path $repo)) { throw "the repository is not staged at $repo" }

# Enumerate the staged test binaries instead of reading a list file: the host
# stages exactly the test executables into target\debug\deps, and a list file is
# one more thing that can go stale (it did: an old list containing a joined
# string made this script try to run all 34 paths as one command).
$deps = Join-Path $repo 'target\debug\deps'
if (-not (Test-Path $deps)) { throw "no staged binaries at $deps" }
$exes = @(Get-ChildItem -Path $deps -Filter '*.exe' | Sort-Object Name | Select-Object -ExpandProperty FullName)
if (-not $exes) { throw "no test binaries in $deps" }

$filter = $Filter
$extra = @('--test-threads=1')
# A `--bless` prefix asks the golden tests to rewrite their reference images
# instead of comparing against them: rustamp-testkit's assert_golden reads
# RUSTAMP_BLESS and writes when it is exactly `1`. Like `--ignored`, the token is
# stripped from the binary-name filter, so `--bless:golden` blesses and runs only
# the golden binaries. Without the token nothing here changes.
$bless = $false
if ($filter -like '--bless*') {
    $bless = $true
    $filter = ($filter -replace '^--bless:?', '')
    $env:RUSTAMP_BLESS = '1'
}
# An `--ignored` prefix asks the binaries to run their ignored tests as well.
# Those are exactly the tests that open a window or spawn a process: they are
# ignored by default because the host must stay windowless, and they may only
# run here, in the guest. `--ignored:name` also filters by binary name.
if ($filter -like '--ignored*') {
    $extra = @('--ignored', '--test-threads=1')
    $filter = ($filter -replace '^--ignored:?', '')
}

$results = @()
$executedCount = 0

# Timeout per binary: the suite's slowest honest binary runs in couple of minutes.
# 300 seconds (5 min) gives generous margin; a binary that outlives it is hung, not slow.
# A hang is a finding about the code under test, not about the runner.
$TestTimeoutSeconds = 300

# Real test media: the host runner stages read-only copies at C:\rustamp\testmedia
# when the owner's archive copy exists on the host. Point the documented
# RUSTAMP_TESTDATA variable at it so media-dependent tests opt in; without it
# they skip, exactly as on a machine without the archive.
if (Test-Path 'C:\rustamp\testmedia') { $env:RUSTAMP_TESTDATA = 'C:\rustamp\testmedia' }
# Real test skins, the same contract: the host runner stages read-only copies at
# C:\rustamp\testskins (`.wsz` files and/or skin folders) when they exist on the
# host. Point RUSTAMP_TESTSKINS at it so skin-dependent tests opt in; without it
# they skip.
if (Test-Path 'C:\rustamp\testskins') { $env:RUSTAMP_TESTSKINS = 'C:\rustamp\testskins' }
# Real test plugins: staged read-only copies at C:\rustamp\testplugins when they
# exist on the host. Point RUSTAMP_TESTPLUGINS at it so DLL acceptance tests find
# the approved in_openmpt.dll plugin.
if (Test-Path 'C:\rustamp\testplugins') { $env:RUSTAMP_TESTPLUGINS = 'C:\rustamp\testplugins' }
# The classic DSP fixture (P8) rides in the same copy at dsp\dsp_optilab_core.dll.
# Point RUSTAMP_TESTPLUGIN_DSP at the file itself when it is there, and hash it so
# the host can confirm the guest holds exactly the pinned bytes (reported below).
$dspPluginPath = 'C:\rustamp\testplugins\dsp\dsp_optilab_core.dll'
$dspPluginHash = $null
if (Test-Path $dspPluginPath) {
    $env:RUSTAMP_TESTPLUGIN_DSP = $dspPluginPath
    $dspPluginHash = (Get-FileHash -Algorithm SHA256 -LiteralPath $dspPluginPath).Hash
}

# Mediahost resident fixture examples: set RUSTAMP_DECODER_FIXTURE and
# RUSTAMP_DSP_FIXTURE when staged, enabling the two-process pcm_process and
# dsp_process ignored tests.
$decFixture = Join-Path $repo 'target\debug\examples\decoder_fixture.exe'
if (-not (Test-Path $decFixture)) {
    $decFixture = Join-Path $repo 'target\debug\decoder_fixture.exe'
}
if (Test-Path $decFixture) { $env:RUSTAMP_DECODER_FIXTURE = $decFixture }

$dspFixture = Join-Path $repo 'target\debug\examples\dsp_fixture.exe'
if (-not (Test-Path $dspFixture)) {
    $dspFixture = Join-Path $repo 'target\debug\dsp_fixture.exe'
}
if (Test-Path $dspFixture) { $env:RUSTAMP_DSP_FIXTURE = $dspFixture }

function Assert-RequiredFixture([string]$Path) {
    if (-not (Test-Path -LiteralPath $Path -PathType Leaf) -or (Get-Item -LiteralPath $Path -ErrorAction Stop).Length -eq 0) {
        throw "required staged fixture missing or empty: $Path; refusing before any test binary runs"
    }
}

function Test-SelectedIgnoredBinary([string]$Pattern) {
    return ($extra -contains '--ignored' -and @($exes | Where-Object {
        $name = Split-Path $_ -Leaf
        $name -like $Pattern -and (-not $filter -or $name -like "*$filter*")
    }).Count -gt 0)
}

if ($extra -notcontains '--ignored' -and @($exes | Where-Object {
    (Split-Path $_ -Leaf) -like 'base291_acceptance-*' -and (-not $filter -or (Split-Path $_ -Leaf) -like "*$filter*")
}).Count -gt 0) {
    $base291Skin = 'C:\rustamp\testskins\base-2.91.wsz'
    Assert-RequiredFixture $base291Skin
    $env:RUSTAMP_BASE291_SKIN = $base291Skin
}

# Detect incomplete copyin before the suite can degenerate into fixture refusals.
# The two DSP fixtures deliberately share an old environment name; the DLL is
# selected per binary below, never substituted for the resident executable.
if ($extra -contains '--ignored' -and -not $desktopFree) {
    foreach ($fixtureName in 'gen_rustamp_fixture', 'out_rustamp_fixture', 'in_rustamp_fixture',
        'in_handoff_rustamp_fixture', 'vis_rustamp_fixture', 'dsp_rustamp_fixture', 'crash_rustamp_fixture') {
        Assert-RequiredFixture (Join-Path $deps "$fixtureName.dll")
    }
    Assert-RequiredFixture (Join-Path $deps 'rustamp-plughost32.exe')
    if ((Test-SelectedIgnoredBinary 'dll_acceptance-*') -or
        (Test-SelectedIgnoredBinary 'control_load_acceptance-*') -or
        (Test-SelectedIgnoredBinary 'rustamp_plughost-*')) {
        $openmpt = @('C:\rustamp\testplugins\in_openmpt.dll', 'C:\rustamp\testplugins\libopenmpt\Winamp\in_openmpt.dll') |
            Where-Object { Test-Path -LiteralPath $_ -PathType Leaf } | Select-Object -First 1
        if (-not $openmpt) { throw 'required staged in_openmpt.dll missing; refusing before any test binary runs' }
        Assert-RequiredFixture $openmpt
    }
    if (Test-SelectedIgnoredBinary 'dsp_acceptance-*') {
        Assert-RequiredFixture $dspPluginPath
        if ($dspPluginHash -ne '78A140C9114B60B361DC7E9A0203DD6D6F23D567E1F309396CD78BAEA7797874') {
            throw 'required staged approved DSP fixture has the wrong SHA-256; refusing before any test binary runs'
        }
    }
}

Remove-Item Env:\RUSTAMP_BASE291_VM_CAPTURES -ErrorAction SilentlyContinue
if (Test-SelectedIgnoredBinary 'base291_acceptance-*') {
    $captureRoot = 'C:\rustamp\base291-captures'
    Assert-RequiredFixture (Join-Path $captureRoot 'session-proof.json')
    foreach ($case in 'main-active', 'main-inactive', 'main-shade', 'main-double',
        'eq-active', 'eq-inactive', 'eq-shade', 'eq-double',
        'playlist-active', 'playlist-inactive', 'playlist-shade') {
        foreach ($kind in 'presented', 'reference') {
            Assert-RequiredFixture (Join-Path $captureRoot "$case.$kind.png")
        }
    }
    $env:RUSTAMP_BASE291_VM_CAPTURES = $captureRoot
    Write-Host "base291 captures: $captureRoot (11 real pairs and capture-session proof present)"
}

# --- launch one test binary, bounded, and bring its output back as text -------
#
# Two rules this helper follows, both written down because the first version of
# the timeout broke both of them:
#
# 1. EVERY value the launched binary needs arrives as a PARAMETER. The first
#    version ran it as `Start-Job -ScriptBlock { & $exe @extra 2>&1 }`: a job
#    body executes in its OWN runspace, where $exe and $extra do not exist, so
#    `& $exe @extra` was `& $null @null` - which is precisely the guest's
#    "invalid object after &" error, and no binary ran at all. Only two values
#    were ever needed and both are explicit parameters here.
# 2. The exit code comes from the process object this script started. A job's
#    $LASTEXITCODE stays inside the job, so the version that read it in the
#    parent was reading a stale value from an earlier native command - which is
#    how a failing binary could come to be reported as a pass.
#
# Start-Process is not an option either: its -PassThru object loses the exit
# code in Windows PowerShell 5.1 (it enables events on the object, which
# releases the process handle), so `$proc.ExitCode` reads back $null. A
# [Diagnostics.Process] started here belongs to this script - its ExitCode, its
# bounded WaitForExit and its Kill all work - which is the same shape
# run-smoke.ps1 already uses for the player.
function Invoke-TestBinary {
    param(
        [Parameter(Mandatory = $true)] [string] $Exe,
        [string[]] $ArgList = @(),
        [Parameter(Mandatory = $true)] [int] $TimeoutSeconds,
        # Bounded wait for the redirected streams to run dry once the binary is
        # gone. A child that leaves a grandchild holding the pipe would hold the
        # read open forever, so this wait is bounded and its failure is reported.
        [int] $DrainMs = 10000
    )

    $notes = @()
    $psi = New-Object Diagnostics.ProcessStartInfo
    $psi.FileName = $Exe
    # ProcessStartInfo takes ONE command-line string, so the argument list has to
    # be quoted per element and joined here. The only argument the runner passes
    # today is the bare `--ignored` flag; the quoting is here so that a future
    # argument containing a space cannot silently become two.
    $psi.Arguments = ((@($ArgList) | ForEach-Object {
            if ($_ -match '\s') { '"' + $_ + '"' } else { $_ }
        }) -join ' ')
    # UseShellExecute=$false is what makes redirection (and therefore captured
    # output) possible at all; CreateNoWindow keeps the run windowless.
    $psi.UseShellExecute = $false
    $psi.CreateNoWindow = $true
    $psi.RedirectStandardOutput = $true
    $psi.RedirectStandardError = $true
    if ((Split-Path $Exe -Leaf) -like 'dsp_fixture_acceptance-*') {
        $pluginDsp = Join-Path $deps 'dsp_rustamp_fixture.dll'
        Assert-RequiredFixture $pluginDsp
        $psi.EnvironmentVariables['RUSTAMP_DSP_FIXTURE'] = $pluginDsp
    }
    # The child inherits this process's environment - RUSTAMP_TESTDATA and
    # RUSTAMP_TESTSKINS, set just above - and the subst'd D:, because it is an
    # ordinary child of this session: nothing is re-established in a new
    # runspace, so nothing can go missing on the way.

    $proc = $null
    try {
        $proc = [Diagnostics.Process]::Start($psi)
    } catch {
        # A binary that will not start is one finding, not the end of the run.
        return [pscustomobject]@{
            started = $false
            hung     = $false
            exit     = -1
            stdout   = ''
            stderr   = ''
            notes    = @(('the binary could not be started: ' + $_.Exception.Message))
        }
    }

    # Drain BOTH pipes on the thread pool from the first moment. A pipe holds
    # only ~64 KB: a chatty child fills it, blocks on write and would be reported
    # as hung although it is merely loud. ReadToEndAsync is why the bound below
    # cannot be defeated by output volume.
    $outTask = $proc.StandardOutput.ReadToEndAsync()
    $errTask = $proc.StandardError.ReadToEndAsync()

    $hung = -not $proc.WaitForExit($TimeoutSeconds * 1000)
    if ($hung) {
        # The object here IS the hung binary, so no second mechanism is needed
        # to reach it - a job or a cmd wrapper would have left the binary running
        # after the runner had given up on it.
        try { $proc.Kill() } catch { $notes += ('the hung binary could not be killed: ' + $_.Exception.Message) }
        if (-not $proc.WaitForExit(10000)) {
            $notes += 'the hung binary was still alive 10 s after Kill; it may still be running in the guest'
        }
    }

    $exit = -1
    if (-not $hung) {
        # Never a default of 0: an unreadable exit code must not look like a pass.
        try { $exit = $proc.ExitCode } catch { $notes += ('the exit code could not be read: ' + $_.Exception.Message) }
    }

    # Whatever the binary managed to print is kept either way - a hung binary's
    # partial output is the only evidence of where it stopped. Wait() THROWS when
    # the task faulted (a pipe broken by the kill, for instance), and a runner
    # that dies on that would report nothing at all, so both drains are guarded.
    $outText = ''
    try {
        if ($outTask.Wait($DrainMs)) { $outText = [string]$outTask.Result } else {
            $notes += ("stdout was still open {0} ms after the binary ended (a child process is probably still holding it); only part of its output could be read" -f $DrainMs)
        }
    } catch {
        $notes += ('stdout could not be read: ' + $_.Exception.Message)
    }
    $errText = ''
    try {
        if ($errTask.Wait($DrainMs)) { $errText = [string]$errTask.Result } else {
            $notes += ("stderr was still open {0} ms after the binary ended (a child process is probably still holding it); only part of its output could be read" -f $DrainMs)
        }
    } catch {
        $notes += ('stderr could not be read: ' + $_.Exception.Message)
    }

    $proc.Dispose()
    return [pscustomobject]@{
        started = $true
        hung     = $hung
        exit     = $exit
        stdout   = $outText
        stderr   = $errText
        notes    = @($notes)
    }
}

foreach ($exe in $exes) {
    $name = Split-Path $exe -Leaf
    if ($filter -and $name -notlike "*$filter*") { continue }
    if ($desktopFree -and $name -notmatch ('^' + [regex]::Escape($filter) + '-[0-9a-f]+\.exe$')) { continue }

    # Skip non-test helper binaries staged beside test binaries for candidate lookup
    if ($name -eq 'rustamp-plughost32.exe' -or $name -eq 'rustamp.exe') { continue }

    # Group E tests: real audio endpoint tests cannot run in headless VM without a
    # hardware/VM redirection decision. When running a general --ignored pass (not
    # specifically filtering for them), leave them out cleanly with diagnostic rather than
    # failing on missing WASAPI device / VB-Audio cable.
    if ($extra -contains '--ignored' -and (-not $filter -or ($filter -notlike '*real_playback*' -and $filter -notlike '*loopback*'))) {
        if ($name -like '*real_playback*') {
            $results += [pscustomobject]@{
                exe            = $name
                exit           = 0
                summary        = 'LEFT OUT: requires VM audio endpoint decision (WASAPI default output)'
                passed         = @()
                failed         = @()
                diagnostic     = @('left out per W12/Group E1: requires Hyper-V audio redirection or virtual audio endpoint decision')
                hung           = $false
                timeoutSeconds = $TestTimeoutSeconds
            }
            continue
        }
        if ($name -like '*loopback*') {
            $results += [pscustomobject]@{
                exe            = $name
                exit           = 0
                summary        = 'LEFT OUT: requires VB-Audio Virtual Cable installation in VM'
                passed         = @()
                failed         = @()
                diagnostic     = @('left out per W12/Group E2: requires VB-Audio Virtual Cable (CABLE Input/Output) decision')
                hung           = $false
                timeoutSeconds = $TestTimeoutSeconds
            }
            continue
        }
    }

    # Progress line: WHICH binary is running right now. It has to come after
    # $name exists and before the binary starts - the first version printed it
    # once, above this loop, with an empty name.
    Write-Host ("running: {0}" -f $name)

    if (-not (Test-Path $exe)) {
        $results += [pscustomobject]@{
            exe            = $name
            exit           = -1
            summary        = 'MISSING BINARY'
            passed         = @()
            failed         = @()
            diagnostic     = @()
            hung           = $false
            timeoutSeconds = $TestTimeoutSeconds
        }
        continue
    }

    $run = Invoke-TestBinary -Exe $exe -ArgList $extra -TimeoutSeconds $TestTimeoutSeconds
    if ($logDir) {
        @($run.stdout, $run.stderr) | Set-Content -LiteralPath (Join-Path $logDir ($name + '.log')) -Encoding UTF8 -ErrorAction Stop
    }

    if (-not $run.started) {
        $results += [pscustomobject]@{
            exe            = $name
            exit           = -1
            summary        = 'DID NOT START'
            passed         = @()
            failed         = @()
            diagnostic     = @($run.notes)
            hung           = $false
            timeoutSeconds = $TestTimeoutSeconds
        }
        continue
    }
    $executedCount++

    # stdout first, then stderr. The two are separate pipes now, so their lines
    # cannot be interleaved the way the old `2>&1` merge appeared to; every match
    # below is a whole-line match, and the panic block the test harness writes to
    # stderr still gets its own window.
    $lines = @()
    foreach ($text in @($run.stdout, $run.stderr)) {
        if ([string]::IsNullOrEmpty($text)) { continue }
        $lines += @($text -split "`r?`n")
    }

    # Hung: record the stall with what it printed, and carry on with the rest.
    if ($run.hung) {
        # The TAIL is what matters - that is where it stopped - and an unbounded
        # dump would bury every other finding in the report.
        $diag = @($lines | Where-Object { $_ -and $_.Trim() } | Select-Object -Last 25)
        if (-not $diag) { $diag = @('no output captured: the binary printed nothing before the timeout') }
        $results += [pscustomobject]@{
            exe            = $name
            exit           = -1
            summary        = 'HUNG after timeout'
            passed         = @()
            failed         = @()
            diagnostic     = @($diag + @($run.notes))
            hung           = $true
            timeoutSeconds = $TestTimeoutSeconds
        }
        continue
    }

    $summary = ($lines | Where-Object { $_ -match '^test result:' } | Select-Object -Last 1)
    $failedNames = @($lines | Where-Object { $_ -match '^test .* FAILED$' } | ForEach-Object {
        (($_ -replace '^test\s+', '') -replace '\s+\.\.\.\s+FAILED$', '') -replace '\s+FAILED$', ''
    })
    $passedNames = @($lines | Where-Object { $_ -match '^test .* \.\.\. ok$' } | ForEach-Object {
        ($_ -replace '^test\s+', '') -replace '\s+\.\.\.\s+ok$', ''
    })
    # A panic's values live on the lines AFTER the FAILED line (multi-line
    # assertion messages), so keep a window behind each failure: without it a
    # failure like "timeline stopped at X of Y frames" keeps its verdict but
    # loses X and Y, and every diagnosis costs another guest round trip.
    $failureWindows = @()
    for ($i = 0; $i -lt $lines.Count; $i++) {
        if ($lines[$i] -match '^test .* FAILED$|panicked at') {
            $failureWindows += @($lines | Select-Object -Skip $i -First 10)
        }
    }
    # When a binary produces no test summary it did not run its tests at all
    # (usually it could not start), so keep the first output lines: without them
    # a failure like this is a mystery.
    $diagnostic = @()
    if (-not $summary) { $diagnostic = @($lines | Where-Object { $_ -and $_.Trim() } | Select-Object -First 5) }
    # A failing test's own output is the only way to see *why* from the runner, and
    # a test that panics says so in one line. Keep those lines (with a little
    # context) so the failure is diagnosable without a second round trip.
    if ($failedNames.Count -gt 0) {
        $interesting = @($lines | Where-Object { $_ -match 'panicked|assertion|FAILED|BLOCKED|left:|right:|Error|error:' } | Select-Object -First 14)
        if ($interesting) { $diagnostic = @($diagnostic + $interesting) }
        if ($failureWindows) { $diagnostic = @($diagnostic + @($failureWindows | Select-Object -First 20)) }
    }
    # Runner-level findings (a kill that did not land, an unreadable exit code)
    # belong with the binary's own output: they explain the verdict above them.
    if ($run.notes) { $diagnostic = @($diagnostic + @($run.notes)) }
    # Starting a program without a test summary, or losing its output, cannot
    # certify a pass even when its process exit code happens to be zero.
    $effectiveExit = $run.exit
    if (-not $summary -or $run.notes) { $effectiveExit = -1 }
    if (-not $summary) { $diagnostic += 'no test result summary was produced; the process exit alone cannot certify a pass' }
    $results += [pscustomobject]@{
        exe            = $name
        exit           = $effectiveExit
        summary        = $summary
        passed         = $passedNames
        failed         = $failedNames
        diagnostic     = $diagnostic
        hung           = $false
        # The test's own 30 s escape is a stall too, not the runner's 300 s
        # timeout. Inspect all output, not the bounded diagnostic excerpt.
        stalled        = [bool](@($lines | Where-Object { $_ -match 'the resident stopped delivering' }).Count)
        timeoutSeconds = $TestTimeoutSeconds
    }
}

# Only with --bless: stage the re-blessed images under C:\rustamp\out\bless, the
# one tree the broker's copyout op may read. Keeping the skin sub-folder means the
# files with the same name cannot overwrite each other. The names are an explicit
# list rather than "every golden the run touched": a bless rewrites every image in
# the golden binaries, and bringing all of them back would promote a wholesale
# re-bless instead of the reviewed images. Files that were not written are simply
# absent; the host checks for them and fails on a missing one.
$blessNames = @('vis_stopped_tail', 'playlist_window_selection', 'playlist_window_mid_page')
if ($bless) {
    $blessOut = 'C:\rustamp\out\bless'
    if (Test-Path $blessOut) { Remove-Item -Recurse -Force $blessOut }
    New-Item -ItemType Directory -Force -Path $blessOut | Out-Null
    foreach ($skin in 'lookalike', 'rust-theme') {
        foreach ($name in $blessNames) {
            $src = Join-Path $repo "tests\golden\$skin\$name.png"
            if (Test-Path $src) {
                $dstDir = Join-Path $blessOut $skin
                New-Item -ItemType Directory -Force -Path $dstDir | Out-Null
                Copy-Item -LiteralPath $src -Destination $dstDir -Force
            }
        }
    }
}

[pscustomobject]@{
    host      = $env:COMPUTERNAME
    user      = $env:USERNAME
    session   = $session
    repo      = $repo
    subst     = (Test-Path 'D:\code\Rustamp')
    # Reports whether the plugin fixture was staged: set only when
    # C:\rustamp\testplugins exists (see above), so $null here means the Group D
    # loader tests ran without RUSTAMP_TESTPLUGINS and can only refuse loudly.
    testplugins = $env:RUSTAMP_TESTPLUGINS
    # Same idea for the DSP fixture: $null means it was not found in the guest.
    testplugindsp = $env:RUSTAMP_TESTPLUGIN_DSP
    testplugindspsha256 = $dspPluginHash
    base291captures = $env:RUSTAMP_BASE291_VM_CAPTURES
    # LEFT OUT / MISSING / DID NOT START rows are not executed binaries.
    ran       = $executedCount
    testsRun  = (@($results | ForEach-Object { $_.passed }).Count + @($results | ForEach-Object { $_.failed }).Count)
    failed    = @($results | Where-Object { $_.exit -ne 0 }).Count
    results   = $results
}
