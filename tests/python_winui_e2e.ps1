#!/usr/bin/env pwsh
# Copyright (c) Microsoft Corporation.
# Licensed under the MIT License.

param(
    [Parameter(Mandatory)]
    [string]$WinuiWinmd,

    [Parameter(Mandatory)]
    [string]$RefList,

    [Parameter(Mandatory)]
    [string]$BootstrapDll,

    [int]$Major = 2,
    [int]$Minor = 3,
    [string]$Python = "python",
    [switch]$SkipBuild
)

$ErrorActionPreference = "Stop"
$caller = (Get-Location).Path
$root = Split-Path $PSScriptRoot -Parent

function Resolve-CallerPath([string]$Path) {
    if ([System.IO.Path]::IsPathRooted($Path)) {
        return [System.IO.Path]::GetFullPath($Path)
    }
    return [System.IO.Path]::GetFullPath((Join-Path $caller $Path))
}

$WinuiWinmd = Resolve-CallerPath $WinuiWinmd
$RefList = Resolve-CallerPath $RefList
$BootstrapDll = Resolve-CallerPath $BootstrapDll
if ([System.IO.Path]::IsPathRooted($Python)) {
    $Python = [System.IO.Path]::GetFullPath($Python)
} else {
    $callerPython = Join-Path $caller $Python
    if (Test-Path -LiteralPath $callerPython -PathType Leaf) {
        $Python = [System.IO.Path]::GetFullPath($callerPython)
    }
}

$output = Join-Path $PSScriptRoot "e2e_generated\python_winui_bindings"
$runner = Join-Path $PSScriptRoot "runners\py_winui_runner.py"
$targetRoot = if ($env:CARGO_TARGET_DIR) {
    if ([System.IO.Path]::IsPathRooted($env:CARGO_TARGET_DIR)) {
        $env:CARGO_TARGET_DIR
    } else {
        Join-Path $root $env:CARGO_TARGET_DIR
    }
} else {
    Join-Path $root "target"
}
$codegen = Join-Path $targetRoot "release\dynwinrt-codegen.exe"

foreach ($path in @($WinuiWinmd, $RefList, $BootstrapDll)) {
    if (-not (Test-Path -LiteralPath $path -PathType Leaf)) {
        throw "Required WinUI E2E input was not found: $path"
    }
}

if (-not $SkipBuild) {
    Push-Location $root
    try {
        cargo build -p dynwinrt-codegen --release
        if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
    } finally {
        Pop-Location
    }

    Push-Location (Join-Path $root "bindings\py")
    try {
        & $Python -m maturin develop --release
        if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
    } finally {
        Pop-Location
    }
}

if (-not (Test-Path -LiteralPath $codegen -PathType Leaf)) {
    throw "dynwinrt-codegen release binary was not found: $codegen"
}

if (Test-Path -LiteralPath $output) {
    Remove-Item -LiteralPath $output -Recurse -Force
}
New-Item -ItemType Directory -Force -Path $output | Out-Null

$generations = @(
    @("Microsoft.UI.Xaml", "Application,Window"),
    @("Microsoft.UI.Xaml.Markup", "XamlReader"),
    @("Microsoft.UI.Xaml.Controls", "Grid,Button,TextBlock,CommandBar,AppBarButton,ItemsRepeater,ListView,StackLayout,StackPanel"),
    @("Microsoft.UI.Xaml.Automation.Peers", "ButtonAutomationPeer"),
    @("Windows.System.Threading", "ThreadPool")
)

foreach ($generation in $generations) {
    & $codegen generate `
        --winmd $WinuiWinmd `
        --ref-list $RefList `
        --namespace $generation[0] `
        --class-name $generation[1] `
        --lang py `
        --output $output
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
}

& $Python $runner `
    --bindings-dir $output `
    --bootstrap-dll $BootstrapDll `
    --major $Major `
    --minor $Minor
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
