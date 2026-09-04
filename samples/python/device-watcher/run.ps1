#!/usr/bin/env pwsh

param(
    [string]$Python = "python",
    [int]$TimeoutSeconds = 10,
    [switch]$ShowNames
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
    (Join-Path $PSScriptRoot "app.py"),
    "--timeout", $TimeoutSeconds
)
if ($ShowNames) {
    $arguments += "--show-names"
}
& $Python @arguments
if ($LASTEXITCODE -ne 0) {
    exit $LASTEXITCODE
}
