#!/usr/bin/env pwsh

param(
    [Parameter(Mandatory)]
    [string]$WinuiWinmd,

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

$WinuiWinmd = Resolve-CallerPath $WinuiWinmd
$RefList = Resolve-CallerPath $RefList
$BootstrapDll = Resolve-CallerPath $BootstrapDll

foreach ($path in @($WinuiWinmd, $RefList, $BootstrapDll)) {
    if (-not (Test-Path -LiteralPath $path -PathType Leaf)) {
        throw "Required input was not found: $path"
    }
}

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
    @("Microsoft.UI.Xaml.Markup", "XamlReader"),
    @("Microsoft.UI.Xaml.Controls", "StackPanel,Button,TextBlock"),
    @("Microsoft.UI.Xaml.Media", "MicaBackdrop")
)

foreach ($generation in $generations) {
    & $codegenCommand generate `
        --winmd $WinuiWinmd `
        --ref-list $RefList `
        --namespace $generation[0] `
        --class-name $generation[1] `
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
