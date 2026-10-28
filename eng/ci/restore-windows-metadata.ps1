# Copyright (c) Microsoft Corporation.
# Licensed under the MIT License.

param(
    [Parameter(Mandatory = $true)][string]$OutputDirectory,
    [string]$NuGet = "nuget"
)

$ErrorActionPreference = "Stop"
$root = (Resolve-Path (Join-Path $PSScriptRoot "..\..")).Path
$pin = Get-Content -LiteralPath (Join-Path $root "tools\dynwinrt-codegen\tests\windows_sdk_corpus.json") -Raw |
    ConvertFrom-Json
$destination = [IO.Path]::GetFullPath($OutputDirectory)
$packageRoot = Join-Path $destination "$($pin.package).$($pin.version)"
$winmd = Join-Path $packageRoot $pin.relative_path
if (-not (Test-Path -LiteralPath $winmd -PathType Leaf)) {
    & $NuGet install $pin.package -Version $pin.version `
        -OutputDirectory $destination -DirectDownload -NonInteractive | Out-Host
    if ($LASTEXITCODE -ne 0) {
        throw "Failed to restore $($pin.package) $($pin.version)"
    }
}
if (-not (Test-Path -LiteralPath $winmd -PathType Leaf)) {
    throw "Pinned Windows SDK package is missing $($pin.relative_path)"
}
$actual = (Get-FileHash -LiteralPath $winmd -Algorithm SHA256).Hash
if ($actual -ne $pin.sha256) {
    throw "Pinned Windows SDK metadata hash mismatch: expected $($pin.sha256), got $actual at $winmd"
}
Write-Host "Verified Windows SDK corpus: $($pin.package) $($pin.version), SHA256 $actual"
(Resolve-Path -LiteralPath $winmd).Path
