param(
    [Parameter(Mandatory = $true)]
    [string]$SourceRepo
)

$ErrorActionPreference = 'Stop'
$RepoRoot = Split-Path -Parent $PSScriptRoot
$OverlayRoot = Join-Path $RepoRoot 'overlay'
$FilesRoot = Join-Path $OverlayRoot 'files\libs\cua-driver\rust'
$PatchPath = Join-Path $OverlayRoot 'tracked.patch'

$expectedHead = 'bf6c76786d938070f4ecf1e44004752f69f518b8'
$actualHead = (& git -C $SourceRepo rev-parse HEAD).Trim()
if ($LASTEXITCODE -ne 0) { throw 'git rev-parse failed' }
if ($actualHead -ne $expectedHead) {
    throw "Expected CUA HEAD $expectedHead, got $actualHead"
}

& git -C $SourceRepo diff --binary --full-index "--output=$PatchPath" HEAD -- 'libs/cua-driver/rust'
if ($LASTEXITCODE -ne 0) { throw 'git diff failed' }

$rustRoot = Join-Path $SourceRepo 'libs\cua-driver\rust'
$files = @(
    'crates\platform-windows\src\bin\private_visual_worker.rs',
    'crates\platform-windows\src\execution_environment.rs',
    'crates\platform-windows\src\execution_environment\private_interference.rs',
    'crates\platform-windows\src\execution_environment\private_visual.rs',
    'crates\platform-windows\src\execution_environment\private_visual_worker_client.rs',
    'crates\platform-windows\src\execution_environment\runtime.rs',
    'crates\platform-windows\src\execution_environment\win32_private.rs',
    'crates\platform-windows\src\execution_environment\winforms_native.rs',
    'crates\platform-windows\tests\private_desktop_semantic.rs',
    'crates\platform-windows\tests\fixtures\WinFormsKeyFixture.cs',
    'crates\platform-windows\tests\fixtures\WpfStableFixture.cs'
)

if (Test-Path $FilesRoot) {
    Remove-Item -Recurse -Force $FilesRoot
}

foreach ($relative in $files) {
    $source = Join-Path $rustRoot $relative
    if (-not (Test-Path -LiteralPath $source -PathType Leaf)) {
        throw "Missing overlay source file: $source"
    }
    $target = Join-Path $FilesRoot $relative
    New-Item -ItemType Directory -Force -Path (Split-Path -Parent $target) | Out-Null
    Copy-Item -LiteralPath $source -Destination $target
}

Write-Host "Captured CUA private-desktop overlay from $actualHead"
