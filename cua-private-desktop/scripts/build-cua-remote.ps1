param(
    [Parameter(Mandatory = $true)]
    [string]$OutputDir,
    [switch]$AllowLocalBuild
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$IsGitHubActions = $env:GITHUB_ACTIONS -eq 'true'
if (-not $IsGitHubActions -and -not $AllowLocalBuild) {
    throw 'Deep CUA build is remote-only by default. Use -AllowLocalBuild only for an explicit local fallback.'
}

$RepoRoot = Split-Path -Parent $PSScriptRoot
$Materializer = Join-Path $PSScriptRoot 'materialize-cua.ps1'
$TempRoot = if ($IsGitHubActions) { $env:RUNNER_TEMP } else { $env:TEMP }
if ([string]::IsNullOrWhiteSpace($TempRoot)) { $TempRoot = [IO.Path]::GetTempPath() }
$BuildRoot = Join-Path $TempRoot 'webgpt-cua-private-desktop'
$BuildSourceCommit = if ($IsGitHubActions -and $env:GITHUB_SHA) { $env:GITHUB_SHA } else { (git -C $RepoRoot rev-parse HEAD).Trim() }
$RustRoot = Join-Path $BuildRoot 'libs\cua-driver\rust'
$Target = 'x86_64-pc-windows-msvc'
$UpstreamVersion = '0.30.4'
$UpstreamCommit = 'bf6c76786d938070f4ecf1e44004752f69f518b8'
$OfficialAssetSha256 = '7b0ec893797fdeb0514d96f5797ad6aa53617eadcba2e47856461f60140c757b'
$OfficialAssetUrl = 'https://github.com/trycua/cua/releases/download/cua-driver-rs-v0.30.4/cua-driver-rs-0.30.4-windows-x86_64-binary.zip'

& $Materializer -Destination $BuildRoot -Force
if ($LASTEXITCODE -ne 0) { throw 'CUA materialization failed' }

git -C $BuildRoot diff --check
if ($LASTEXITCODE -ne 0) { throw 'materialized overlay diff check failed' }

Push-Location $RustRoot
try {
    rustup run 1.97.1 cargo fmt --all -- --check
    if ($LASTEXITCODE -ne 0) { throw 'cargo fmt failed' }

    rustup run 1.97.1 cargo metadata --no-deps --locked --format-version 1 | Out-Null
    if ($LASTEXITCODE -ne 0) { throw 'cargo metadata failed' }

    rustup run 1.97.1 cargo check -p cua-driver-contract -p cua-driver-core -p platform-windows --locked
    if ($LASTEXITCODE -ne 0) { throw 'targeted cargo check failed' }

    # Exercise the highest-risk WebGPT integration immediately after the
    # compile gate so semantic regressions fail before lower-risk unit suites.
    rustup run 1.97.1 cargo test -p platform-windows --test private_desktop_semantic --locked -- --test-threads=1
    if ($LASTEXITCODE -ne 0) { throw 'private desktop semantic tests failed' }

    rustup run 1.97.1 cargo test -p cua-driver-contract --lib --locked
    if ($LASTEXITCODE -ne 0) { throw 'contract tests failed' }

    rustup run 1.97.1 cargo test -p cua-driver-core --lib --locked
    if ($LASTEXITCODE -ne 0) { throw 'core tests failed' }

    rustup run 1.97.1 cargo test -p platform-windows --lib --locked
    if ($LASTEXITCODE -ne 0) { throw 'platform-windows unit tests failed' }

    rustup run 1.97.1 cargo test -p cua-driver-sdk abi::tests:: --lib --locked
    if ($LASTEXITCODE -ne 0) { throw 'SDK ABI tests failed' }

    rustup run 1.97.1 cargo build --locked -p cua-driver --release --target $Target
    if ($LASTEXITCODE -ne 0) { throw 'cua-driver release build failed' }

    rustup run 1.97.1 cargo build --locked -p platform-windows --bin private_visual_worker --release --target $Target
    if ($LASTEXITCODE -ne 0) { throw 'private visual worker release build failed' }
}
finally {
    Pop-Location
}

New-Item -ItemType Directory -Force -Path $OutputDir | Out-Null
Copy-Item (Join-Path $RustRoot "target\$Target\release\cua-driver.exe") (Join-Path $OutputDir 'cua-driver.exe') -Force
Copy-Item (Join-Path $RustRoot "target\$Target\release\private_visual_worker.exe") (Join-Path $OutputDir 'private_visual_worker.exe') -Force

$OfficialZip = Join-Path $TempRoot 'cua-driver-0.30.4-official.zip'
$OfficialExtract = Join-Path $TempRoot 'cua-driver-0.30.4-official'
Invoke-WebRequest -Uri $OfficialAssetUrl -OutFile $OfficialZip
$actualOfficialSha = (Get-FileHash -Algorithm SHA256 -LiteralPath $OfficialZip).Hash.ToLowerInvariant()
if ($actualOfficialSha -ne $OfficialAssetSha256) {
    throw "Official CUA asset SHA256 mismatch: $actualOfficialSha"
}
if (Test-Path $OfficialExtract) { Remove-Item -Recurse -Force $OfficialExtract }
Expand-Archive -LiteralPath $OfficialZip -DestinationPath $OfficialExtract -Force

foreach ($name in @('cua-driver-uia.exe', 'cua-cursor-theme.exe')) {
    $source = Get-ChildItem -LiteralPath $OfficialExtract -File -Recurse -Filter $name | Select-Object -First 1
    if (-not $source) { throw "Official asset missing $name" }
    Copy-Item -LiteralPath $source.FullName -Destination (Join-Path $OutputDir $name) -Force
}

$overlaySha = (Get-FileHash -Algorithm SHA256 -LiteralPath (Join-Path $RepoRoot 'overlay\tracked.patch')).Hash.ToLowerInvariant()
$cargoLockSha = (Get-FileHash -Algorithm SHA256 -LiteralPath (Join-Path $RustRoot 'Cargo.lock')).Hash.ToLowerInvariant()
$artifacts = @()
foreach ($name in @('cua-driver.exe', 'private_visual_worker.exe', 'cua-driver-uia.exe', 'cua-cursor-theme.exe')) {
    $path = Join-Path $OutputDir $name
    $item = Get-Item -LiteralPath $path
    $artifacts += [ordered]@{
        name = $name
        sha256 = (Get-FileHash -Algorithm SHA256 -LiteralPath $path).Hash.ToLowerInvariant()
        bytes = [int64]$item.Length
    }
}

$receipt = [ordered]@{
    schemaVersion = 1
    upstreamVersion = $UpstreamVersion
    upstreamCommit = $UpstreamCommit
    buildSourceCommit = $BuildSourceCommit
    buildEnvironment = if ($IsGitHubActions) { 'github-actions' } else { 'local-explicit-fallback' }
    overlayPatchSha256 = $overlaySha
    cargoLockSha256 = $cargoLockSha
    officialWindowsX64AssetSha256 = $OfficialAssetSha256
    rustc = ((rustup run 1.97.1 rustc -Vv) -join [Environment]::NewLine)
    cargo = (rustup run 1.97.1 cargo -V)
    tests = [ordered]@{
        contract = 'passed'
        core = 'passed'
        platformWindows = 'passed'
        sdkAbi = 'passed'
        privateDesktopSemantic = 'passed'
    }
    artifacts = $artifacts
}
$receipt | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath (Join-Path $OutputDir 'build-receipt.json') -Encoding utf8NoBOM
Write-Host "CUA build completed: $OutputDir"
