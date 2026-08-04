#!/usr/bin/env pwsh

param(
    [string]$Python = "python"
)

$ErrorActionPreference = "Stop"
$generated = Join-Path $PSScriptRoot "generated\__init__.py"
$bootstrap = Join-Path $PSScriptRoot ".runtime\Microsoft.WindowsAppRuntime.Bootstrap.dll"

if (-not (Test-Path -LiteralPath $generated -PathType Leaf)) {
    throw "Generated bindings were not found. Run generate.ps1 first."
}
if (-not (Test-Path -LiteralPath $bootstrap -PathType Leaf)) {
    throw "The WinAppSDK bootstrap DLL was not found. Run generate.ps1 first."
}

if (-not [System.IO.Path]::IsPathRooted($Python)) {
    $callerPython = Join-Path (Get-Location).Path $Python
    if (Test-Path -LiteralPath $callerPython -PathType Leaf) {
        $Python = [System.IO.Path]::GetFullPath($callerPython)
    }
}

& $Python (Join-Path $PSScriptRoot "app.py")
if ($LASTEXITCODE -ne 0) {
    exit $LASTEXITCODE
}
