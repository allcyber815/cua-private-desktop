param(
    [Parameter(Mandatory = $true)]
    [string]$Destination,
    [switch]$Force
)

$ErrorActionPreference = 'Stop'
$RepoRoot = Split-Path -Parent $PSScriptRoot
$Pin = 'a9baa8d107fba8b0aef5a4ed6233e498e88d14d0'
$Upstream = 'https://github.com/trycua/cua.git'
$Overlay = Join-Path $RepoRoot 'overlay'

if (Test-Path $Destination) {
    if (-not $Force) {
        throw "Destination already exists: $Destination"
    }
    Remove-Item -LiteralPath $Destination -Recurse -Force
}

New-Item -ItemType Directory -Force -Path $Destination | Out-Null

git init $Destination
if ($LASTEXITCODE -ne 0) { throw 'git init failed' }

git -C $Destination remote add origin $Upstream
if ($LASTEXITCODE -ne 0) { throw 'git remote add failed' }

# Keep ordinary materialization source-only and small: fetch one exact commit
# without blobs, then hydrate the driver plus the local path-dependency workspaces
# required by CUA 0.33.1 metadata/format validation.
git -C $Destination -c protocol.version=2 fetch --depth=1 --filter=blob:none origin $Pin
if ($LASTEXITCODE -ne 0) { throw 'filtered git fetch failed' }

git -C $Destination sparse-checkout init --cone
if ($LASTEXITCODE -ne 0) { throw 'sparse-checkout init failed' }

git -C $Destination sparse-checkout set libs/cua-driver/rust libs/cua libs/fleet
if ($LASTEXITCODE -ne 0) { throw 'sparse-checkout set failed' }

git -C $Destination checkout --detach FETCH_HEAD
if ($LASTEXITCODE -ne 0) { throw 'sparse checkout failed' }

git -C $Destination apply --check (Join-Path $Overlay 'tracked.patch')
if ($LASTEXITCODE -ne 0) { throw 'overlay preflight failed' }

git -C $Destination apply (Join-Path $Overlay 'tracked.patch')
if ($LASTEXITCODE -ne 0) { throw 'overlay apply failed' }

$FilesRoot = Join-Path $Overlay 'files'
Get-ChildItem -Path $FilesRoot -File -Recurse | ForEach-Object {
    $relative = $_.FullName.Substring($FilesRoot.Length).TrimStart('\')
    $target = Join-Path $Destination $relative
    New-Item -ItemType Directory -Force -Path (Split-Path -Parent $target) | Out-Null
    Copy-Item -LiteralPath $_.FullName -Destination $target
}

Write-Host "Materialized CUA private-desktop overlay at $Destination"
