#!/usr/bin/env pwsh

param(
    [string]$Python = "python",
    [string]$Symbol,
    [int]$Limit = 25
)

$ErrorActionPreference = "Stop"
if (-not (Test-Path -LiteralPath (Join-Path $PSScriptRoot "generated\__init__.py"))) {
    throw "Generated bindings were not found. Run generate.ps1 first."
}
if (-not [System.IO.Path]::IsPathRooted($Python)) {
    $callerPython = Join-Path (Get-Location).Path $Python
    if (Test-Path -LiteralPath $callerPython -PathType Leaf) {
        $Python = [System.IO.Path]::GetFullPath($callerPython)
    }
}

$arguments = @(
    (Join-Path $PSScriptRoot "inspect_generated.py"),
    "--limit", $Limit
)
if ($Symbol) {
    $arguments += @("--symbol", $Symbol)
}
& $Python @arguments
if ($LASTEXITCODE -ne 0) {
    exit $LASTEXITCODE
}
