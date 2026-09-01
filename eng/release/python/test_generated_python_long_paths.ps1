# Copyright (c) Microsoft Corporation.
# Licensed under the MIT License.

param(
    [Parameter(Mandatory = $true)][string]$Python,
    [Parameter(Mandatory = $true)][string]$Codegen
)

$ErrorActionPreference = "Stop"
$tempRoot = [System.IO.Path]::GetTempPath().TrimEnd("\")
$prefix = "dynwinrt-python-long-path-"
$paddingLength = 104 - $tempRoot.Length - 1 - $prefix.Length
if ($paddingLength -lt 8) {
    throw "Temporary directory is too long for the generated Python path test"
}
$longSegment = $prefix + ("x" * $paddingLength)
$root = Join-Path $tempRoot $longSegment
if ($root.Length -ne 104) {
    throw "Long-path test root must be 104 characters, got $($root.Length)"
}
$source = Join-Path $root "source\generated_bindings"
$wheelDirectory = Join-Path $root "wheels"
$install = Join-Path $root "installed"

try {
    Remove-Item -LiteralPath $root -Recurse -Force -ErrorAction SilentlyContinue
    New-Item -ItemType Directory -Force -Path $wheelDirectory | Out-Null

    & $Codegen generate `
        --class-name Windows.Devices.Enumeration.DeviceInformationCustomPairing `
        --lang py `
        --output $source
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

    $modules = Get-ChildItem -LiteralPath $source -File -Filter *.py
    $tooLong = $modules | Where-Object { $_.BaseName.Length -gt 120 }
    if ($tooLong) {
        throw "Generated Python module exceeds 120 characters: $($tooLong[0].Name)"
    }
    if (-not ($modules | Where-Object { $_.BaseName -match "_[0-9a-f]{16}$" })) {
        throw "Long-path fixture did not generate a hashed Python module"
    }

    $targetSymbol =
        "TypedEventHandler_DeviceInformationCustomPairing_DevicePairingSetMembersRequestedEventArgs"
    $shortModule = $modules |
        Where-Object {
            $_.BaseName -match "_[0-9a-f]{16}$" -and
            (Select-String -LiteralPath $_.FullName -Pattern $targetSymbol -Quiet)
        } |
        Select-Object -First 1
    if (-not $shortModule) {
        throw "Long-path fixture did not generate the expected typed event handler"
    }

    $buildBaseMatch = Select-String `
        -LiteralPath (Join-Path $source "setup.cfg") `
        -Pattern "^build-base\s*=\s*(.+)$"
    if (-not $buildBaseMatch) {
        throw "Generated setup.cfg does not define a scoped build cache"
    }
    $initialBuildBase = $buildBaseMatch.Matches[0].Groups[1].Value.Trim()
    $initialScopedCache = Join-Path $source $initialBuildBase
    $cachedPackages = @(
        (Join-Path $source "build\lib\generated_bindings"),
        (Join-Path $initialScopedCache "lib\generated_bindings"),
        (Join-Path $source ".venv\Lib\site-packages\generated_bindings"),
        (Join-Path $source "venv\Lib\site-packages\generated_bindings"),
        (Join-Path $source "env\Lib\site-packages\generated_bindings"),
        (Join-Path $source ".tox\py\Lib\site-packages\generated_bindings"),
        (Join-Path $source ".nox\py\Lib\site-packages\generated_bindings")
    )
    foreach ($cachedPackage in $cachedPackages) {
        New-Item -ItemType Directory -Force -Path $cachedPackage | Out-Null
        foreach ($extension in @("py", "pyi")) {
            Copy-Item `
                -LiteralPath (Join-Path $source "$($shortModule.BaseName).$extension") `
                -Destination $cachedPackage
        }
    }
    $eggInfo = Join-Path $source "generated_bindings.egg-info"
    New-Item -ItemType Directory -Force -Path $eggInfo | Out-Null
    Set-Content -LiteralPath (Join-Path $eggInfo "SOURCES.txt") -Value $shortModule.BaseName
    $dist = Join-Path $source "dist"
    New-Item -ItemType Directory -Force -Path $dist | Out-Null
    Set-Content -LiteralPath (Join-Path $dist "old-generated-bindings.whl") -Value $shortModule.BaseName
    $pycache = Join-Path $source "__pycache__"
    New-Item -ItemType Directory -Force -Path $pycache | Out-Null
    Set-Content -LiteralPath (Join-Path $pycache "old.pyc") -Value $shortModule.BaseName
    $preservedArtifacts = @(
        (Join-Path $source "build"),
        $initialScopedCache,
        (Join-Path $source ".venv"),
        (Join-Path $source "venv"),
        (Join-Path $source "env"),
        (Join-Path $source ".tox"),
        (Join-Path $source ".nox"),
        $dist,
        $pycache,
        $eggInfo
    )

    & $Codegen generate `
        --class-name Windows.Foundation.Uri `
        --lang py `
        --output $source
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
    foreach ($artifact in $preservedArtifacts) {
        if (-not (Test-Path -LiteralPath $artifact)) {
            throw "Incremental generation deleted user artifact $artifact"
        }
    }
    $newBuildBaseMatch = Select-String `
        -LiteralPath (Join-Path $source "setup.cfg") `
        -Pattern "^build-base\s*=\s*(.+)$"
    $newBuildBase = $newBuildBaseMatch.Matches[0].Groups[1].Value.Trim()
    if ($newBuildBase -eq $initialBuildBase) {
        throw "Generated file layout change did not rotate the scoped build cache"
    }
    foreach ($extension in @("py", "pyi")) {
        if (-not (Test-Path -LiteralPath (
            Join-Path $source "$($shortModule.BaseName).$extension"))) {
            throw "Unrelated incremental generation removed $($shortModule.BaseName).$extension"
        }
    }

    $preRenameBuildBase = $newBuildBase
    $renamedSource = Join-Path (Split-Path $source -Parent) "renamed_bindings"
    Move-Item -LiteralPath $source -Destination $renamedSource
    $source = $renamedSource
    $staleOldPackage = Join-Path $source (
        Join-Path $preRenameBuildBase "lib\generated_bindings")
    New-Item -ItemType Directory -Force -Path $staleOldPackage | Out-Null
    Set-Content `
        -LiteralPath (Join-Path $staleOldPackage "stale_old_package.py") `
        -Value "STALE = True"
    Set-Content `
        -LiteralPath (Join-Path $staleOldPackage "stale_old_package.pyi") `
        -Value "STALE: bool"

    & $Codegen generate `
        --class-name Windows.Foundation.Uri `
        --lang py `
        --output $source
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
    $renamedBuildBaseMatch = Select-String `
        -LiteralPath (Join-Path $source "setup.cfg") `
        -Pattern "^build-base\s*=\s*(.+)$"
    $renamedBuildBase = $renamedBuildBaseMatch.Matches[0].Groups[1].Value.Trim()
    if ($renamedBuildBase -eq $preRenameBuildBase) {
        throw "Output package rename did not rotate the scoped build cache"
    }
    if (-not (Test-Path -LiteralPath $staleOldPackage)) {
        throw "Output package rename deleted the old build cache"
    }

    & $Python -m pip wheel $source --no-deps --wheel-dir $wheelDirectory
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
    $wheel = Get-ChildItem -LiteralPath $wheelDirectory -Filter *.whl -File |
        Select-Object -First 1
    if (-not $wheel) { throw "Generated bindings wheel was not created" }

    & $Python -m pip install $wheel.FullName --no-deps --target $install
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
    & $Python (Join-Path $PSScriptRoot "verify_generated_python_install.py") `
        --source $source `
        --install $install `
        --package renamed_bindings
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
    if (Test-Path -LiteralPath (Join-Path $install "generated_bindings")) {
        throw "Renamed wheel retained the old generated_bindings package"
    }
} finally {
    Remove-Item -LiteralPath $root -Recurse -Force -ErrorAction SilentlyContinue
}
