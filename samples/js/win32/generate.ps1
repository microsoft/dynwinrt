#!/usr/bin/env pwsh

param(
    [Parameter(Mandatory)]
    [string]$Win32Winmd,

    [string]$Codegen = "dynwinrt-codegen"
)

$ErrorActionPreference = "Stop"
$caller = (Get-Location).Path

function Resolve-CallerPath([string]$Path) {
    if ([System.IO.Path]::IsPathRooted($Path)) {
        return [System.IO.Path]::GetFullPath($Path)
    }
    return [System.IO.Path]::GetFullPath((Join-Path $caller $Path))
}

$Win32Winmd = Resolve-CallerPath $Win32Winmd
if (-not (Test-Path -LiteralPath $Win32Winmd -PathType Leaf)) {
    throw "Windows.Win32.winmd was not found: $Win32Winmd"
}

$codegenCandidate = Resolve-CallerPath $Codegen
if (Test-Path -LiteralPath $codegenCandidate -PathType Leaf) {
    $codegenCommand = $codegenCandidate
} else {
    $codegenCommand = (Get-Command $Codegen -ErrorAction Stop).Source
}

$output = Join-Path $PSScriptRoot "generated"
if (Test-Path -LiteralPath $output) {
    Remove-Item -LiteralPath $output -Recurse -Force
}

foreach ($namespace in @(
    "Windows.Win32.System.SystemInformation",
    "Windows.Win32.System.Registry",
    "Windows.Win32.Storage.FileSystem"
)) {
    & $codegenCommand generate `
        --winmd $Win32Winmd `
        --namespace $namespace `
        --class-name Apis `
        --output $output
    if ($LASTEXITCODE -ne 0) {
        throw "dynwinrt-codegen failed for $namespace with exit code $LASTEXITCODE"
    }
}

Write-Host "Generated flat Win32 bindings in $output"
