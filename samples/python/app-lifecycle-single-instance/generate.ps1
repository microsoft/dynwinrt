#!/usr/bin/env pwsh

param(
    [Parameter(Mandatory)]
    [string]$AppLifecycleWinmd,

    [Parameter(Mandatory)]
    [string]$RefList,

    [Parameter(Mandatory)]
    [string]$BootstrapDll,

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

$AppLifecycleWinmd = Resolve-CallerPath $AppLifecycleWinmd
$RefList = Resolve-CallerPath $RefList
$BootstrapDll = Resolve-CallerPath $BootstrapDll
foreach ($path in @($AppLifecycleWinmd, $RefList, $BootstrapDll)) {
    if (-not (Test-Path -LiteralPath $path -PathType Leaf)) {
        throw "Required input was not found: $path"
    }
}

$codegenCandidate = Resolve-CallerPath $Codegen
if (Test-Path -LiteralPath $codegenCandidate -PathType Leaf) {
    $codegenCommand = $codegenCandidate
} else {
    $codegenCommand = (Get-Command $Codegen -ErrorAction Stop).Source
}

$output = Join-Path $PSScriptRoot "generated"
$runtime = Join-Path $PSScriptRoot ".runtime"
if (Test-Path -LiteralPath $output) {
    Remove-Item -LiteralPath $output -Recurse -Force
}
New-Item -ItemType Directory -Force -Path $output, $runtime | Out-Null

& $codegenCommand generate `
    --winmd $AppLifecycleWinmd `
    --ref-list $RefList `
    --class-name "Microsoft.Windows.AppLifecycle.AppInstance" `
    --lang py `
    --output $output
if ($LASTEXITCODE -ne 0) {
    throw "dynwinrt-codegen failed with exit code $LASTEXITCODE"
}

Copy-Item -LiteralPath $BootstrapDll `
    -Destination (Join-Path $runtime "Microsoft.WindowsAppRuntime.Bootstrap.dll") `
    -Force

Write-Host "Generated AppLifecycle bindings in $output"
Write-Host "Copied the bootstrap DLL to $runtime"
