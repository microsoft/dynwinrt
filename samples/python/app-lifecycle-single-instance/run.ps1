#!/usr/bin/env pwsh

param(
    [string]$Python = "python",
    [int]$Major = 2,
    [int]$Minor = 3
)

$ErrorActionPreference = "Stop"
if (-not (Test-Path -LiteralPath (Join-Path $PSScriptRoot "generated\__init__.py"))) {
    throw "Generated bindings were not found. Run generate.ps1 first."
}
if (-not (Test-Path -LiteralPath (Join-Path $PSScriptRoot ".runtime\Microsoft.WindowsAppRuntime.Bootstrap.dll"))) {
    throw "The WinAppSDK bootstrap DLL was not found. Run generate.ps1 first."
}
if (-not [System.IO.Path]::IsPathRooted($Python)) {
    $callerPython = Join-Path (Get-Location).Path $Python
    if (Test-Path -LiteralPath $callerPython -PathType Leaf) {
        $Python = [System.IO.Path]::GetFullPath($callerPython)
    }
}

& $Python (Join-Path $PSScriptRoot "app.py") `
    --loopback `
    --major $Major `
    --minor $Minor
if ($LASTEXITCODE -ne 0) {
    exit $LASTEXITCODE
}
