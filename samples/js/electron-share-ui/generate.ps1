# Copyright (c) Microsoft Corporation.
# Licensed under the MIT License.

$ErrorActionPreference = "Stop"

$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..\..\..")).Path
$output = Join-Path $PSScriptRoot "generated"

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
    Remove-Item $output -Recurse -Force
}

& cargo run --quiet --manifest-path (Join-Path $repoRoot "Cargo.toml") `
    -p dynwinrt-codegen -- generate `
    --winmd $windowsWinmd `
    --namespace Windows.ApplicationModel.DataTransfer `
    --class-name DataTransferManager `
    --output $output
if ($LASTEXITCODE -ne 0) {
    throw "WinRT generation failed with exit code $LASTEXITCODE."
}

& cargo run --quiet --manifest-path (Join-Path $repoRoot "Cargo.toml") `
    -p dynwinrt-codegen -- generate `
    --winmd $win32Winmd `
    --ref $windowsWinmd `
    --namespace Windows.Win32.UI.Shell `
    --class-name IDataTransferManagerInterop `
    --output $output
if ($LASTEXITCODE -ne 0) {
    throw "Classic COM generation failed with exit code $LASTEXITCODE."
}

Write-Host "Generated Share UI bindings in $output"

