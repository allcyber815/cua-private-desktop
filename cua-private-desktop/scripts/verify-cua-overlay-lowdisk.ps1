param(
    [string]$Destination,
    [switch]$Keep
)

$ErrorActionPreference = 'Stop'
$RepoRoot = Split-Path -Parent $PSScriptRoot
$Materializer = Join-Path $PSScriptRoot 'materialize-cua.ps1'
if (-not $Destination) {
    $Destination = Join-Path $RepoRoot '.deps\cua-lowdisk-verify'
}

try {
    & $Materializer -Destination $Destination -Force
    if ($LASTEXITCODE -ne 0) { throw 'CUA overlay materialization failed' }

    git -C $Destination diff --check
    if ($LASTEXITCODE -ne 0) { throw 'git diff --check failed' }

    $RustRoot = Join-Path $Destination 'libs\cua-driver\rust'

    foreach ($requiredSourceAsset in @(
        (Join-Path $Destination 'libs\images\sandbox-images.json'),
        (Join-Path $Destination 'libs\cua\crates\cua-teleport\src\ux\targets.json')
    )) {
        if (-not (Test-Path -LiteralPath $requiredSourceAsset)) {
            throw "required CUA 0.33.1 compile-time source asset missing: $requiredSourceAsset"
        }
    }

    # The upstream checkout currently pins a Rust toolchain that may not be
    # installed locally. Explicit +stable prevents rustup from downloading that
    # historical toolchain during ordinary source-only verification.
    Push-Location $RustRoot
    try {
        cargo +stable metadata --no-deps --locked --format-version 1 | Out-Null
        if ($LASTEXITCODE -ne 0) { throw 'cargo metadata failed' }

        cargo +stable fmt --all -- --check
        if ($LASTEXITCODE -ne 0) { throw 'cargo fmt --check failed' }
    }
    finally {
        Pop-Location
    }

    $unexpected = @(
        (Join-Path $RustRoot 'target'),
        (Join-Path $Destination '.cargo-home-lowdisk'),
        (Join-Path $Destination '.target-lowdisk')
    ) | Where-Object { Test-Path -LiteralPath $_ }

    if ($unexpected.Count -ne 0) {
        throw "low-disk verification created forbidden build/cache state: $($unexpected -join ', ')"
    }

    Write-Host 'CUA low-disk overlay verification passed.'
}
finally {
    if (-not $Keep -and (Test-Path -LiteralPath $Destination)) {
        Remove-Item -LiteralPath $Destination -Recurse -Force
    }
}
