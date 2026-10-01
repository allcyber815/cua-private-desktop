param(
    [switch]$ForcePrepare,
    [switch]$ForceMaterialize,
    [string]$TestFilter = ""
)

$ErrorActionPreference = 'Stop'
$RepoRoot = Split-Path -Parent $PSScriptRoot
$FixtureRoot = Join-Path $RepoRoot '.deps\private-desktop-fixtures'
$CuaRoot = Join-Path $RepoRoot '.deps\cua-private-desktop'
$TargetRoot = Join-Path $RepoRoot '.deps\cua-target'
$CargoHomeRoot = Join-Path $RepoRoot '.deps\cua-cargo-home'

$prepare = Join-Path $PSScriptRoot 'prepare-private-desktop-fixtures.ps1'
if ($ForcePrepare -or -not (Test-Path -LiteralPath (Join-Path $FixtureRoot 'fixture-manifest.json'))) {
    & $prepare -RuntimeRoot $FixtureRoot -Force:$ForcePrepare
    if ($LASTEXITCODE -ne 0) { throw "Fixture preparation failed with exit code $LASTEXITCODE" }
}

if ($ForceMaterialize -or -not (Test-Path -LiteralPath $CuaRoot)) {
    & (Join-Path $PSScriptRoot 'materialize-cua.ps1') -Destination $CuaRoot -Force:$ForceMaterialize
    if ($LASTEXITCODE -ne 0) { throw "CUA materialization failed with exit code $LASTEXITCODE" }
}

$PythonwCandidates = @(
    "$env:LOCALAPPDATA\Python\bin\pythonw.exe",
    "$env:LOCALAPPDATA\Programs\Python\Python313\pythonw.exe",
    "$env:LOCALAPPDATA\Programs\Python\Python312\pythonw.exe"
)
$Pythonw = $PythonwCandidates | Where-Object { Test-Path -LiteralPath $_ } | Select-Object -First 1
if (-not $Pythonw) { throw 'pythonw.exe was not found.' }

$Chrome = "$env:ProgramFiles\Google\Chrome\Application\chrome.exe"
if (-not (Test-Path -LiteralPath $Chrome -PathType Leaf)) { throw 'Chrome executable was not found.' }

$Electron = Join-Path $FixtureRoot 'electron\dist\electron.exe'
$WinformsInvoke = Join-Path $FixtureRoot 'BgFixture.exe'
$WinformsNative = Join-Path $FixtureRoot 'BgFixtureV3.exe'
foreach ($required in @($Electron, $WinformsInvoke, $WinformsNative)) {
    if (-not (Test-Path -LiteralPath $required -PathType Leaf)) {
        throw "Prepared fixture missing: $required"
    }
}

$env:WEBGPT_CUA_CONFORMANCE_ROOT = $FixtureRoot
$env:WEBGPT_CUA_WINFORMS_INVOKE_FIXTURE = $WinformsInvoke
$env:WEBGPT_CUA_WINFORMS_NATIVE_FIXTURE = $WinformsNative
$env:WEBGPT_CUA_PYTHONW_EXE = $Pythonw
$env:WEBGPT_CUA_ELECTRON_EXE = $Electron
$env:WEBGPT_CUA_CHROME_EXE = $Chrome
$env:CARGO_HOME = $CargoHomeRoot
$env:CARGO_TARGET_DIR = $TargetRoot
$env:CARGO_INCREMENTAL = '0'
$env:CARGO_PROFILE_DEV_DEBUG = '0'

$RustRoot = Join-Path $CuaRoot 'libs\cua-driver\rust'
Push-Location $RustRoot
try {
    $baseArgs = @(
        '+stable',
        'test',
        '--locked',
        '-p', 'platform-windows',
        '--test', 'private_desktop_semantic'
    )

    # These integration cases create independent bare registries inside one
    # process-wide runtime/session environment. Keep them serialized so test
    # fixture bookkeeping cannot cross-contaminate another case.
    $coreArgs = @($baseArgs) + @('--', '--test-threads=1')
    & cargo @coreArgs
    if ($LASTEXITCODE -ne 0) {
        throw "Private desktop core tests failed with exit code $LASTEXITCODE"
    }

    $providerArgs = @($baseArgs)
    if (-not [string]::IsNullOrWhiteSpace($TestFilter)) {
        $providerArgs += $TestFilter
    }
    $providerArgs += @('--', '--ignored', '--test-threads=1')
    & cargo @providerArgs
    if ($LASTEXITCODE -ne 0) {
        throw "Private desktop provider conformance tests failed with exit code $LASTEXITCODE"
    }
} finally {
    Pop-Location
}

Write-Host 'Private desktop conformance tests passed.'
