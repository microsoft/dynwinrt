#!/usr/bin/env pwsh

param(
    [Parameter(Mandatory)]
    [string]$AppNotificationsWinmd,

    [Parameter(Mandatory)]
    [string]$BuilderWinmd,

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

$AppNotificationsWinmd = Resolve-CallerPath $AppNotificationsWinmd
$BuilderWinmd = Resolve-CallerPath $BuilderWinmd
$RefList = Resolve-CallerPath $RefList
$BootstrapDll = Resolve-CallerPath $BootstrapDll
foreach ($path in @(
    $AppNotificationsWinmd,
    $BuilderWinmd,
    $RefList,
    $BootstrapDll
)) {
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
    --winmd "$AppNotificationsWinmd;$BuilderWinmd" `
    --ref-list $RefList `
    --class-name "Microsoft.Windows.AppNotifications.AppNotificationManager,Microsoft.Windows.AppNotifications.Builder.AppNotificationBuilder" `
    --lang py `
    --output $output
if ($LASTEXITCODE -ne 0) {
    throw "dynwinrt-codegen failed with exit code $LASTEXITCODE"
}

Copy-Item -LiteralPath $BootstrapDll `
    -Destination (Join-Path $runtime "Microsoft.WindowsAppRuntime.Bootstrap.dll") `
    -Force

Write-Host "Generated AppNotification bindings in $output"
Write-Host "Copied the bootstrap DLL to $runtime"
