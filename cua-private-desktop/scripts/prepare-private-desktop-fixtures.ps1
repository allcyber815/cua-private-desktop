param(
    [string]$RuntimeRoot = "",
    [switch]$Force
)

$ErrorActionPreference = 'Stop'
$RepoRoot = Split-Path -Parent $PSScriptRoot
$SourceRoot = Join-Path $RepoRoot 'test-fixtures\private-desktop'
if ([string]::IsNullOrWhiteSpace($RuntimeRoot)) {
    $RuntimeRoot = Join-Path $RepoRoot '.deps\private-desktop-fixtures'
}
$RuntimeRoot = [IO.Path]::GetFullPath($RuntimeRoot)
$ManifestPath = Join-Path $RuntimeRoot 'fixture-manifest.json'

$Pins = [ordered]@{
    schema = 1
    webview2 = '1.0.3179.45'
    windowsAppSdk = '1.8.260317003'
    windowsSdkBuildTools = '10.0.26100.7175'
    pyside6Essentials = '6.11.2'
    electron = '44.4.2'
}

if ((Test-Path -LiteralPath $ManifestPath) -and -not $Force) {
    $current = Get-Content -LiteralPath $ManifestPath -Raw | ConvertFrom-Json
    $matches = (
        [int]$current.schema -eq $Pins.schema -and
        [string]$current.webview2 -eq $Pins.webview2 -and
        [string]$current.windowsAppSdk -eq $Pins.windowsAppSdk -and
        [string]$current.windowsSdkBuildTools -eq $Pins.windowsSdkBuildTools -and
        [string]$current.pyside6Essentials -eq $Pins.pyside6Essentials -and
        [string]$current.electron -eq $Pins.electron
    )
    if ($matches) {
        Write-Host "Private desktop fixture runtime is current: $RuntimeRoot"
        exit 0
    }
}

if (Test-Path -LiteralPath $RuntimeRoot) {
    if (-not $Force) {
        throw "Fixture runtime exists but does not match the pinned manifest. Re-run with -Force: $RuntimeRoot"
    }
    Remove-Item -LiteralPath $RuntimeRoot -Recurse -Force
}
New-Item -ItemType Directory -Force -Path $RuntimeRoot | Out-Null

# Keep heavy provider setup disposable. dotnet/NuGet and npm caches live under
# the prepared fixture runtime so removing that runtime also recovers downloads.
$env:NUGET_PACKAGES = Join-Path $RuntimeRoot '.nuget-packages'
$env:npm_config_cache = Join-Path $RuntimeRoot '.npm-cache'
$env:npm_config_update_notifier = 'false'

$BinRoot = $RuntimeRoot

function Invoke-Checked {
    param(
        [Parameter(Mandatory = $true)][scriptblock]$Command,
        [Parameter(Mandatory = $true)][string]$Label
    )
    & $Command
    if ($LASTEXITCODE -ne 0) {
        throw "$Label failed with exit code $LASTEXITCODE"
    }
}

$cscCandidates = @(
    "$env:WINDIR\Microsoft.NET\Framework64\v4.0.30319\csc.exe",
    "$env:WINDIR\Microsoft.NET\Framework\v4.0.30319\csc.exe"
)
$Csc = $cscCandidates | Where-Object { Test-Path -LiteralPath $_ } | Select-Object -First 1
if (-not $Csc) { throw 'Windows .NET Framework C# compiler was not found.' }
$FrameworkRoot = Split-Path -Parent $Csc

Invoke-Checked -Label 'BgFixture compile' -Command {
    & $Csc /nologo /target:winexe "/out:$BinRoot\BgFixture.exe" /reference:System.dll /reference:System.Drawing.dll /reference:System.Windows.Forms.dll (Join-Path $SourceRoot 'BgFixture.cs')
}
Invoke-Checked -Label 'BgFixtureV3 compile' -Command {
    & $Csc /nologo /target:winexe "/out:$BinRoot\BgFixtureV3.exe" /reference:System.dll /reference:System.Drawing.dll /reference:System.Windows.Forms.dll (Join-Path $SourceRoot 'BgFixtureV3.cs')
}
Invoke-Checked -Label 'PrivateDesktopUiaDumpWorker compile' -Command {
    $uiaClient = Join-Path $FrameworkRoot 'WPF\UIAutomationClient.dll'
    $uiaTypes = Join-Path $FrameworkRoot 'WPF\UIAutomationTypes.dll'
    & $Csc /nologo /target:exe "/out:$BinRoot\PrivateDesktopUiaDumpWorker.exe" /reference:System.dll "/reference:$uiaClient" "/reference:$uiaTypes" (Join-Path $SourceRoot 'PrivateDesktopUiaDumpWorker.cs')
}

$WinUiOut = Join-Path $RuntimeRoot 'winui-private-fixture\bin\x64\Debug\net10.0-windows10.0.26100.0'
$XamlIslandOut = Join-Path $RuntimeRoot 'xaml-island-private-fixture\bin\x64\Debug\net10.0-windows10.0.17763.0\win-x64'
$WebView2Out = Join-Path $RuntimeRoot 'webview2-private-fixture'
$IntermediateRoot = Join-Path $RuntimeRoot 'obj'
$WinUiObj = (Join-Path $IntermediateRoot 'winui') + '\'
$XamlIslandObj = (Join-Path $IntermediateRoot 'xaml-island') + '\'
$WebView2Obj = (Join-Path $IntermediateRoot 'webview2') + '\'
Invoke-Checked -Label 'WinUI fixture build' -Command {
    & dotnet build (Join-Path $SourceRoot 'winui\winui-private-fixture.csproj') -c Debug -p:Platform=x64 "-p:BaseIntermediateOutputPath=$WinUiObj" "-p:MSBuildProjectExtensionsPath=$WinUiObj" -o $WinUiOut
}
Invoke-Checked -Label 'XAML Island fixture build' -Command {
    & dotnet build (Join-Path $SourceRoot 'xaml-island\XamlIslandPrivateFixture.csproj') -c Debug -p:Platform=x64 -r win-x64 "-p:BaseIntermediateOutputPath=$XamlIslandObj" "-p:MSBuildProjectExtensionsPath=$XamlIslandObj" -o $XamlIslandOut
}
Invoke-Checked -Label 'WebView2 fixture build' -Command {
    & dotnet build (Join-Path $SourceRoot 'webview2\WebView2PrivateFixture.csproj') -c Debug -p:Platform=x64 -r win-x64 "-p:BaseIntermediateOutputPath=$WebView2Obj" "-p:MSBuildProjectExtensionsPath=$WebView2Obj" -o $WebView2Out
}

$PythonCandidates = @(
    "$env:LOCALAPPDATA\Python\bin\python.exe",
    "$env:LOCALAPPDATA\Programs\Python\Python313\python.exe",
    "$env:LOCALAPPDATA\Programs\Python\Python312\python.exe"
)
$Python = $PythonCandidates | Where-Object { Test-Path -LiteralPath $_ } | Select-Object -First 1
if (-not $Python) {
    $PythonCommand = Get-Command python.exe -ErrorAction SilentlyContinue
    if ($PythonCommand) { $Python = $PythonCommand.Source }
}
if (-not $Python) { throw 'Python was not found for the Qt fixture runtime.' }
$QtRuntime = Join-Path $RuntimeRoot 'qt-python-runtime'
Invoke-Checked -Label 'PySide6-Essentials fixture install' -Command {
    & $Python -m pip install --disable-pip-version-check --no-cache-dir --target $QtRuntime "PySide6-Essentials==$($Pins.pyside6Essentials)"
}

$NpmCandidates = @(
    "$env:ProgramFiles\nodejs\npm.cmd",
    "$env:APPDATA\npm\npm.cmd"
)
$Npm = $NpmCandidates | Where-Object { Test-Path -LiteralPath $_ } | Select-Object -First 1
if (-not $Npm) {
    $NpmCommand = Get-Command npm.cmd -ErrorAction SilentlyContinue
    if ($NpmCommand) { $Npm = $NpmCommand.Source }
}
if (-not $Npm) { throw 'npm.cmd was not found for the Electron fixture runtime.' }

$ElectronInstall = Join-Path $RuntimeRoot '.electron-install'
$ElectronCache = Join-Path $RuntimeRoot '.npm-cache'
New-Item -ItemType Directory -Force -Path $ElectronInstall | Out-Null
@{
    private = $true
    devDependencies = @{ electron = $Pins.electron }
} | ConvertTo-Json -Depth 4 | Set-Content -LiteralPath (Join-Path $ElectronInstall 'package.json') -Encoding UTF8

Push-Location $ElectronInstall
try {
    Invoke-Checked -Label 'Electron package install' -Command {
        & $Npm install --no-audit --no-fund --cache $ElectronCache
    }
    $Node = Join-Path (Split-Path -Parent $Npm) 'node.exe'
    if (-not (Test-Path -LiteralPath $Node -PathType Leaf)) {
        throw "node.exe was not found next to npm.cmd: $Node"
    }
    $previousElectronCache = $env:electron_config_cache
    $env:electron_config_cache = $ElectronCache
    try {
        Invoke-Checked -Label 'Electron binary install' -Command {
            & $Node (Join-Path $ElectronInstall 'node_modules\electron\install.js')
        }
    } finally {
        if ($null -eq $previousElectronCache) {
            Remove-Item Env:electron_config_cache -ErrorAction SilentlyContinue
        } else {
            $env:electron_config_cache = $previousElectronCache
        }
    }
} finally {
    Pop-Location
}
$ElectronDist = Join-Path $RuntimeRoot 'electron\dist'
New-Item -ItemType Directory -Force -Path (Split-Path -Parent $ElectronDist) | Out-Null
Copy-Item -LiteralPath (Join-Path $ElectronInstall 'node_modules\electron\dist') -Destination $ElectronDist -Recurse
Remove-Item -LiteralPath $ElectronInstall -Recurse -Force
if (Test-Path -LiteralPath $ElectronCache) { Remove-Item -LiteralPath $ElectronCache -Recurse -Force }

Copy-Item -LiteralPath (Join-Path $SourceRoot 'electron-app') -Destination (Join-Path $RuntimeRoot 'electron-private-app') -Recurse
Copy-Item -LiteralPath (Join-Path $SourceRoot 'chromium-private-fixture.html') -Destination (Join-Path $RuntimeRoot 'chromium-private-fixture.html')
Copy-Item -LiteralPath (Join-Path $SourceRoot 'tk-private-fixture.py') -Destination (Join-Path $RuntimeRoot 'tk-private-fixture.py')

$Pins | ConvertTo-Json | Set-Content -LiteralPath $ManifestPath -Encoding UTF8
Write-Host "Prepared private desktop fixture runtime: $RuntimeRoot"
