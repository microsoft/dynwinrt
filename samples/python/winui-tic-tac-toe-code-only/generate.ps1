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
    -PrimaryWinmdNames "Microsoft.UI.Xaml.winmd" `
    -Architecture $Architecture
$WinuiWinmd = $inputs.PrimaryWinmds["Microsoft.UI.Xaml.winmd"]
$RefList = $inputs.RefList
$BootstrapDll = $inputs.BootstrapDll

$codegenCandidate = Resolve-CallerPath $Codegen
if (Test-Path -LiteralPath $codegenCandidate -PathType Leaf) {
    $codegenCommand = $codegenCandidate
} else {
    $command = Get-Command $Codegen -ErrorAction Stop
    $codegenCommand = $command.Source
}

$output = Join-Path $PSScriptRoot "generated"
$runtime = Join-Path $PSScriptRoot ".runtime"
if (Test-Path -LiteralPath $output) {
    Remove-Item -LiteralPath $output -Recurse -Force
}
New-Item -ItemType Directory -Force -Path $output, $runtime | Out-Null

$generations = @(
    @("Microsoft.UI.Xaml", "Application,Window"),
    @("Microsoft.UI.Xaml.Controls", "StackPanel,Grid,RowDefinition,ColumnDefinition,Button,TextBlock"),
    @("Microsoft.UI.Xaml.Automation", "AutomationProperties"),
    @("Microsoft.UI.Xaml.Media", "MicaBackdrop")
)

foreach ($generation in $generations) {
    & $codegenCommand generate `
        --winmd $WinuiWinmd `
        --ref-list $RefList `
        --namespace $generation[0] `
        --class-name $generation[1] `
        --lang py `
        --output $output
    if ($LASTEXITCODE -ne 0) {
        throw "dynwinrt-codegen failed with exit code $LASTEXITCODE"
    }
}

Copy-Item -LiteralPath $BootstrapDll `
    -Destination (Join-Path $runtime "Microsoft.WindowsAppRuntime.Bootstrap.dll") `
    -Force

Write-Host "Generated bindings in $output"
Write-Host "Copied the bootstrap DLL to $runtime"
