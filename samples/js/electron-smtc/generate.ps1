# Copyright (c) Microsoft Corporation.
# Licensed under the MIT License.

$ErrorActionPreference = "Stop"

$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..\..\..")).Path
$output = Join-Path $PSScriptRoot "generated"

function Remove-GeneratedOutput([string]$Path) {
    if (-not (Test-Path -LiteralPath $Path)) {
        return
    }
    $fullPath = [System.IO.Path]::GetFullPath($Path)
    $extendedPath = if ($fullPath.StartsWith("\\")) {
        "\\?\UNC\" + $fullPath.TrimStart("\")
    } else {
        "\\?\$fullPath"
    }
    [System.IO.Directory]::Delete($extendedPath, $true)
}

$windowsWinmd = Get-ChildItem `
    "C:\Program Files (x86)\Windows Kits\10\UnionMetadata" `
    -Filter Windows.winmd `
    -Recurse `
    -ErrorAction SilentlyContinue |
    Sort-Object FullName -Descending |
    Select-Object -First 1 -ExpandProperty FullName

if (-not $windowsWinmd) {
    throw "Windows.winmd was not found. Install a Windows 10/11 SDK."
}

$win32Winmd = $env:DYNWINRT_WIN32_WINMD
if (-not $win32Winmd -or -not (Test-Path $win32Winmd)) {
    $win32Winmd = Get-ChildItem `
        (Join-Path $env:USERPROFILE ".nuget\packages\microsoft.windows.sdk.win32metadata") `
        -Filter Windows.Win32.winmd `
        -Recurse `
        -ErrorAction SilentlyContinue |
        Sort-Object FullName -Descending |
        Select-Object -First 1 -ExpandProperty FullName
}

if (-not $win32Winmd) {
    throw "Windows.Win32.winmd was not found. Set DYNWINRT_WIN32_WINMD or install Microsoft.Windows.SDK.Win32Metadata."
}

if (Test-Path $output) {
    Remove-GeneratedOutput $output
}

& cargo run --quiet --manifest-path (Join-Path $repoRoot "Cargo.toml") `
    -p dynwinrt-codegen -- generate `
    --winmd $windowsWinmd `
    --class-name "Windows.Media.SystemMediaTransportControls,Windows.Media.Control.GlobalSystemMediaTransportControlsSessionManager" `
    --output $output
if ($LASTEXITCODE -ne 0) {
    throw "WinRT generation failed with exit code $LASTEXITCODE."
}

& cargo run --quiet --manifest-path (Join-Path $repoRoot "Cargo.toml") `
    -p dynwinrt-codegen -- generate `
    --winmd $win32Winmd `
    --ref $windowsWinmd `
    --class-name Windows.Win32.System.WinRT.ISystemMediaTransportControlsInterop `
    --output $output
if ($LASTEXITCODE -ne 0) {
    throw "Classic COM generation failed with exit code $LASTEXITCODE."
}

Write-Host "Generated SMTC and GSMTC bindings in $output"
