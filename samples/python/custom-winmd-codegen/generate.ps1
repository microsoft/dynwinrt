#!/usr/bin/env pwsh

param(
    [Parameter(Mandatory)]
    [string]$Winmd,

    [Parameter(Mandatory)]
    [string]$Namespace,

    [string]$ClassName,
    [string]$RefList,
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

$Winmd = Resolve-CallerPath $Winmd
if (-not (Test-Path -LiteralPath $Winmd -PathType Leaf)) {
    throw "WinMD was not found: $Winmd"
}
if ($RefList) {
    $RefList = Resolve-CallerPath $RefList
    if (-not (Test-Path -LiteralPath $RefList -PathType Leaf)) {
        throw "Reference list was not found: $RefList"
    }
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

$arguments = @(
    "generate",
    "--winmd", $Winmd,
    "--namespace", $Namespace,
    "--lang", "py",
    "--output", $output
)
if ($ClassName) {
    $arguments += @("--class-name", $ClassName)
}
if ($RefList) {
    $arguments += @("--ref-list", $RefList)
}

& $codegenCommand @arguments
if ($LASTEXITCODE -ne 0) {
    throw "dynwinrt-codegen failed with exit code $LASTEXITCODE"
}

Write-Host "Generated custom WinMD bindings in $output"
