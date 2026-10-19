# Host-only regression checks for runner helpers. No Cargo, broker, DLL load,
# GUI, or guest suite is invoked. Synthetic sentinels are never guest fixtures.
# Run with Windows PowerShell 5.1: powershell -File <this script>.
$ErrorActionPreference = 'Stop'
$hostRunner = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..\..\vm-run-tests.ps1'))
$guestRunner = Join-Path $PSScriptRoot 'run-tests.ps1'
$tokens = $null
$errors = $null
$hostAst = [Management.Automation.Language.Parser]::ParseFile($hostRunner, [ref]$tokens, [ref]$errors)
if ($errors) { throw ($errors -join "`n") }
$guestAst = [Management.Automation.Language.Parser]::ParseFile($guestRunner, [ref]$tokens, [ref]$errors)
if ($errors) { throw ($errors -join "`n") }
foreach ($name in 'Assert-RequiredFile', 'Copy-RequiredFile', 'Assert-I686Pe', 'Assert-Base291Captures',
    'Assert-SeparateFixturePaths', 'Get-Inventory', 'Assert-Inventory', 'Test-SelectedIgnoredBinary') {
    $definition = $hostAst.Find({ param($node)
        $node -is [Management.Automation.Language.FunctionDefinitionAst] -and $node.Name -eq $name
    }, $true)
    if (-not $definition) { throw "host helper missing: $name" }
    . ([scriptblock]::Create($definition.Extent.Text))
}
$guestDefinition = $guestAst.Find({ param($node)
    $node -is [Management.Automation.Language.FunctionDefinitionAst] -and $node.Name -eq 'Assert-RequiredFixture'
}, $true)
. ([scriptblock]::Create($guestDefinition.Extent.Text))
$checks = 0
function Assert-Check([bool]$Condition, [string]$Message) {
    if (-not $Condition) { throw $Message }
    $script:checks++
}
function Assert-Refuses([scriptblock]$Action, [string]$Message) {
    $refused = $false
    try { & $Action } catch { $refused = $true }
    Assert-Check $refused $Message
}

$scratch = Join-Path $PSScriptRoot ('fixture-staging-selftest-' + [guid]::NewGuid().ToString('N'))
try {
    $null = New-Item -ItemType Directory -Path $scratch
    $source = Join-Path $scratch 'fixture.dll'
    $destination = Join-Path $scratch 'staged'
    $null = New-Item -ItemType Directory -Path $destination
    Assert-Refuses { Copy-RequiredFile $source $destination } 'missing fixture did not refuse'
    [IO.File]::WriteAllBytes($source, [byte[]]@())
    Assert-Refuses { Copy-RequiredFile $source $destination } 'empty fixture did not refuse'
    [byte[]]$bytes = New-Object byte[] 128
    $bytes[0] = 0x4d; $bytes[1] = 0x5a; $bytes[0x3c] = 64
    $bytes[64] = 0x50; $bytes[65] = 0x45; $bytes[68] = 0x4c; $bytes[69] = 1
    [IO.File]::WriteAllBytes($source, $bytes)
    Assert-I686Pe $source
    $checks++
    Copy-RequiredFile $source $destination
    Assert-Check ((Get-FileHash $source).Hash -eq (Get-FileHash (Join-Path $destination 'fixture.dll')).Hash) 'copy bytes differ'
    $bytes[68] = 0x64; $bytes[69] = 0x86
    [IO.File]::WriteAllBytes($source, $bytes)
    Assert-Refuses { Assert-I686Pe $source } 'x64 fixture did not refuse'
    $bytes[0x3c] = 127
    [IO.File]::WriteAllBytes($source, $bytes)
    Assert-Refuses { Assert-I686Pe $source } 'invalid PE offset did not refuse'
    Assert-Refuses { Assert-RequiredFixture (Join-Path $scratch 'missing.dll') } 'guest missing fixture did not refuse'
    Assert-SeparateFixturePaths (Join-Path $scratch 'source') (Join-Path $scratch 'destination')
    $checks++
    Assert-Refuses { Assert-SeparateFixturePaths $scratch $scratch } 'same capture source/destination did not refuse'
    Assert-Refuses { Assert-SeparateFixturePaths (Join-Path $scratch 'nested') $scratch } 'source nested under removed staging did not refuse'
    Assert-Refuses { Assert-SeparateFixturePaths $scratch (Join-Path $scratch 'nested') } 'staging nested under source did not refuse'

    $captureSource = Join-Path $scratch 'captures'
    $null = New-Item -ItemType Directory -Path $captureSource
    Assert-Refuses { Assert-Base291Captures $captureSource } 'missing capture proof did not refuse'
    Set-Content -LiteralPath (Join-Path $captureSource 'session-proof.json') -Value '{"syntheticSelftestOnly":true}'
    foreach ($case in 'main-active', 'main-inactive', 'main-shade', 'main-double',
        'eq-active', 'eq-inactive', 'eq-shade', 'eq-double',
        'playlist-active', 'playlist-inactive', 'playlist-shade') {
        foreach ($kind in 'presented', 'reference') {
            Set-Content -LiteralPath (Join-Path $captureSource "$case.$kind.png") -Value 'selftest sentinel, not an image'
        }
    }
    Assert-Base291Captures $captureSource
    $checks++
    $captureCopy = Join-Path $scratch 'capture-copy'
    Copy-Item -LiteralPath $captureSource -Destination $captureCopy -Recurse
    Assert-Inventory @(Get-Inventory @(Get-ChildItem $captureSource -File) $captureSource) `
        @(Get-Inventory @(Get-ChildItem $captureCopy -File) $captureCopy) 'capture'
    $checks++
    Remove-Item -LiteralPath (Join-Path $captureCopy 'eq-double.reference.png')
    Assert-Refuses { Assert-Base291Captures $captureCopy } 'missing reference pair did not refuse'
    Assert-Refuses {
        Assert-Inventory @(Get-Inventory @(Get-ChildItem $captureSource -File) $captureSource) `
            @(Get-Inventory @(Get-ChildItem $captureCopy -File) $captureCopy) 'capture'
    } 'incomplete capture inventory did not refuse'

    $selected = @([pscustomobject]@{ Name = 'base291_acceptance-012345.exe' })
    $Filter = '--ignored'
    Assert-Check (Test-SelectedIgnoredBinary 'base291_acceptance-*') 'unfiltered ignored capture selection missed'
    $Filter = '--ignored:base291_acceptance'
    Assert-Check (Test-SelectedIgnoredBinary 'base291_acceptance-*') 'filtered ignored capture selection missed'
    $Filter = ''
    Assert-Check (-not (Test-SelectedIgnoredBinary 'base291_acceptance-*')) 'normal suite incorrectly requires captures'
    $Filter = '--ignored:gen_acceptance'
    $selected = @([pscustomobject]@{ Name = 'gen_acceptance-012345.exe' })
    Assert-Check (-not (Test-SelectedIgnoredBinary 'base291_acceptance-*')) 'unrelated ignored selection requires captures'

    # Exercise the actual per-child environment assignment without starting a
    # process: its AST clause ends before Diagnostics.Process.Start is reached.
    $dspClause = $guestAst.Find({ param($node)
        $node -is [Management.Automation.Language.IfStatementAst] -and
        $node.Extent.Text.StartsWith("if ((Split-Path `$Exe -Leaf) -like 'dsp_fixture_acceptance-*')")
    }, $true)
    Assert-Check ($null -ne $dspClause) 'per-child DSP fixture selection missing'
    $deps = $destination
    Copy-Item -LiteralPath $source -Destination (Join-Path $deps 'dsp_rustamp_fixture.dll')
    $psi = New-Object Diagnostics.ProcessStartInfo
    $Exe = Join-Path $deps 'dsp_fixture_acceptance-012345.exe'
    . ([scriptblock]::Create($dspClause.Extent.Text))
    Assert-Check ($psi.EnvironmentVariables['RUSTAMP_DSP_FIXTURE'] -eq (Join-Path $deps 'dsp_rustamp_fixture.dll')) 'DSP acceptance inherited resident EXE'
    $psi = New-Object Diagnostics.ProcessStartInfo
    $psi.EnvironmentVariables['RUSTAMP_DSP_FIXTURE'] = 'resident-fixture.exe'
    $Exe = Join-Path $deps 'dsp_process-012345.exe'
    . ([scriptblock]::Create($dspClause.Extent.Text))
    Assert-Check ($psi.EnvironmentVariables['RUSTAMP_DSP_FIXTURE'] -eq 'resident-fixture.exe') 'resident fixture environment was overwritten'

    # Exercise the production build and staging loops, not a second mechanism.
    # Cargo is replaced by a recorder; no compiler or generated DLL is run.
    $buildLoop = $hostAst.Find({ param($node)
        $node -is [Management.Automation.Language.ForEachStatementAst] -and
        $node.Extent.Text.StartsWith('foreach ($fixtureName in $pluginFixtureNames)') -and
        $node.Extent.Text.Contains('cargo build --offline')
    }, $true)
    $stageLoop = $hostAst.Find({ param($node)
        $node -is [Management.Automation.Language.ForEachStatementAst] -and
        $node.Extent.Text.StartsWith('foreach ($fixtureName in $pluginFixtureNames)') -and
        $node.Extent.Text.Contains('Copy-RequiredFile $fixtureDll')
    }, $true)
    Assert-Check ($null -ne $buildLoop -and $null -ne $stageLoop) 'production fixture loops missing'
    function Step($Text) { }
    function cargo {
        $script:cargoCalls += ,@($args)
        $global:LASTEXITCODE = $script:cargoExit
    }
    $RepoRoot = Join-Path $scratch 'workspace'
    $Stage = Join-Path $scratch 'stage'
    $examples = Join-Path $RepoRoot 'target\i686-pc-windows-msvc\debug\examples'
    $i686TargetDeps = Join-Path $Stage 'target\i686-pc-windows-msvc\debug\deps'
    $null = New-Item -ItemType Directory -Force -Path $examples, $i686TargetDeps, (Join-Path $Stage 'target\debug\deps')
    $pluginFixtureNames = @('gen_rustamp_fixture', 'out_rustamp_fixture', 'in_rustamp_fixture',
        'in_handoff_rustamp_fixture', 'vis_rustamp_fixture', 'dsp_rustamp_fixture', 'crash_rustamp_fixture')
    $bytes[0x3c] = 64; $bytes[68] = 0x4c; $bytes[69] = 1
    foreach ($fixtureName in $pluginFixtureNames) {
        [IO.File]::WriteAllBytes((Join-Path $examples "$fixtureName.dll"), $bytes)
    }
    $script:cargoCalls = @()
    $script:cargoExit = 0
    . ([scriptblock]::Create($buildLoop.Extent.Text))
    Assert-Check ($script:cargoCalls.Count -eq 7) 'not all seven cdylibs explicitly built'
    foreach ($call in $script:cargoCalls) {
        Assert-Check (($call -join ' ') -like 'build --offline -p rustamp-plughost --example * --target i686-pc-windows-msvc') 'wrong fixture build package/target/command'
    }
    . ([scriptblock]::Create($stageLoop.Extent.Text))
    Assert-Check (@(Get-ChildItem (Join-Path $Stage 'target\debug\deps') -Filter '*.dll').Count -eq 7) 'beside-binaries fixture staging incomplete'
    Assert-Check (@(Get-ChildItem $i686TargetDeps -Filter '*.dll').Count -eq 7) 'i686 layout fixture staging incomplete'
    $script:cargoExit = 71
    Assert-Refuses { . ([scriptblock]::Create($buildLoop.Extent.Text)) } 'failed fixture build did not stop staging'
    $script:cargoExit = 0
    Remove-Item -LiteralPath (Join-Path $examples 'gen_rustamp_fixture.dll')
    Assert-Refuses { . ([scriptblock]::Create($buildLoop.Extent.Text)) } 'successful Cargo exit with missing DLL did not refuse'
    Assert-Refuses { . ([scriptblock]::Create($stageLoop.Extent.Text)) } 'missing source fixture did not stop staging'
    Write-Output "fixture staging self-test: $checks checks passed; no guest operation or binary execution"
} finally {
    if (Test-Path -LiteralPath $scratch) { Remove-Item -LiteralPath $scratch -Recurse -Force }
}
