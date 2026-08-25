#!/usr/bin/env pwsh

param(
    [string]$Python = "python",
    [switch]$Watch,
    [switch]$SkipIdentity
)

$ErrorActionPreference = "Stop"
if (-not (Test-Path -LiteralPath (Join-Path $PSScriptRoot "generated\__init__.py"))) {
    throw "Generated bindings were not found. Run generate.ps1 first."
}
if (-not [System.IO.Path]::IsPathRooted($Python)) {
    $callerPython = Join-Path (Get-Location).Path $Python
    if (Test-Path -LiteralPath $callerPython -PathType Leaf) {
        $Python = [System.IO.Path]::GetFullPath($callerPython)
    } else {
        $Python = (Get-Command $Python -ErrorAction Stop).Source
    }
}
if (-not $SkipIdentity) {
    & (Join-Path $PSScriptRoot "setup-identity.ps1") -Python $Python
}

$arguments = @((Join-Path $PSScriptRoot "app.py"))
if ($Watch) {
    $arguments += "--watch"
}
& $Python @arguments
if ($LASTEXITCODE -ne 0) {
    exit $LASTEXITCODE
}
