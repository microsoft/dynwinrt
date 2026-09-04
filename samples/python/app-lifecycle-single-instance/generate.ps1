#!/usr/bin/env pwsh

param(
    [ValidateSet("x64", "arm64")]
    [string]$Architecture = "x64",

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

. (Join-Path $PSScriptRoot "..\Resolve-WinAppSdkInputs.ps1")
$inputs = Resolve-DynWinRTWinAppSdkInputs `
    -SampleRoot $PSScriptRoot `
    -PrimaryWinmdNames "Microsoft.Windows.AppLifecycle.winmd" `
    -Architecture $Architecture
$AppLifecycleWinmd = $inputs.PrimaryWinmds["Microsoft.Windows.AppLifecycle.winmd"]
$RefList = $inputs.RefList
$BootstrapDll = $inputs.BootstrapDll

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
