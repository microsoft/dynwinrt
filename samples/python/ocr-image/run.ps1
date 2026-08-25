#!/usr/bin/env pwsh

param(
    [string]$Python = "python",
    [string]$Image
)

$ErrorActionPreference = "Stop"
if (-not (Test-Path -LiteralPath (Join-Path $PSScriptRoot "generated\__init__.py"))) {
    throw "Generated bindings were not found. Run generate.ps1 first."
}
if (-not $Image) {
    $Image = Join-Path $PSScriptRoot "sample.png"
    & (Join-Path $PSScriptRoot "make-test-image.ps1") -Output $Image
}
$Image = [System.IO.Path]::GetFullPath($Image)
if (-not (Test-Path -LiteralPath $Image -PathType Leaf)) {
    throw "Image was not found: $Image"
}
if (-not [System.IO.Path]::IsPathRooted($Python)) {
    $callerPython = Join-Path (Get-Location).Path $Python
    if (Test-Path -LiteralPath $callerPython -PathType Leaf) {
        $Python = [System.IO.Path]::GetFullPath($callerPython)
    }
}

& $Python (Join-Path $PSScriptRoot "app.py") --image $Image
if ($LASTEXITCODE -ne 0) {
    exit $LASTEXITCODE
}
