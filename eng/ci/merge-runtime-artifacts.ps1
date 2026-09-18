# Copyright (c) Microsoft Corporation.
# Licensed under the MIT License.

param(
    [Parameter(Mandatory)][string]$X64,
    [Parameter(Mandatory)][string]$Arm64,
    [Parameter(Mandatory)][string]$Output
)

$ErrorActionPreference = "Stop"
& (Join-Path $PSScriptRoot "artifact.ps1") -Mode Verify -Path $X64 -Kind js -Architecture x64
& (Join-Path $PSScriptRoot "artifact.ps1") -Mode Verify -Path $Arm64 -Kind js -Architecture arm64
$left = (Get-Content -LiteralPath (Join-Path $X64 "manifest.json") -Raw | ConvertFrom-Json -AsHashtable).files
$right = (Get-Content -LiteralPath (Join-Path $Arm64 "manifest.json") -Raw | ConvertFrom-Json -AsHashtable).files
$sidecars = @($left.Keys | Where-Object { $_ -notlike "*.node" } | Sort-Object)
$otherSidecars = @($right.Keys | Where-Object { $_ -notlike "*.node" } | Sort-Object)
if (@(Compare-Object $sidecars $otherSidecars).Count) {
    throw "Runtime architecture artifacts have different sidecar file sets."
}
foreach ($file in $sidecars) {
    if ($left[$file] -cne $right[$file]) { throw "Runtime sidecar differs between architectures: $file" }
}
if (Test-Path -LiteralPath $Output) { throw "Runtime assembly output must not already exist: $Output" }
New-Item -ItemType Directory -Path $Output -Force | Out-Null
Copy-Item -Path (Join-Path $X64 "payload\*") -Destination $Output -Recurse
Copy-Item -LiteralPath (Join-Path $Arm64 "payload\dynwinrt.win32-arm64-msvc.node") -Destination $Output
