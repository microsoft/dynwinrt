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
    throw "Windows.winmd was not found. Install a Windows 11 SDK."
}

$aionWinmd = $env:DYNWINRT_AION_WINMD
if (-not $aionWinmd -or -not (Test-Path -LiteralPath $aionWinmd)) {
    $framework = Get-AppxPackage `
        -Name "Microsoft.AionInstructPreview.Framework.1.0" `
        -ErrorAction SilentlyContinue |
        Where-Object Architecture -eq Arm64 |
        Sort-Object Version -Descending |
        Select-Object -First 1
    if ($framework) {
        $aionWinmd = Get-ChildItem `
            -LiteralPath $framework.InstallLocation `
            -Filter AionInstructPreview.Text.winmd `
            -Recurse `
            -ErrorAction SilentlyContinue |
            Select-Object -First 1 -ExpandProperty FullName
    }
}

if (-not $aionWinmd -or -not (Test-Path -LiteralPath $aionWinmd)) {
    $nugetRoot = if ($env:NUGET_PACKAGES) {
        $env:NUGET_PACKAGES
    } else {
        Join-Path $env:USERPROFILE ".nuget\packages"
    }
    $aionWinmd = Get-ChildItem `
        (Join-Path $nugetRoot "aioninstructpreview.text.framework") `
        -Filter AionInstructPreview.Text.winmd `
        -Recurse `
        -ErrorAction SilentlyContinue |
        Sort-Object FullName -Descending |
        Select-Object -First 1 -ExpandProperty FullName
}

if (-not $aionWinmd) {
    throw @"
AionInstructPreview.Text.winmd was not found.
Install Aion Instruct Preview SDK 1.0.0.0 from:
https://github.com/microsoft/Aion-Instruct-Preview-Sample
or set DYNWINRT_AION_WINMD to the SDK WinMD path.
"@
}

if (Test-Path -LiteralPath $output) {
    Remove-Item -LiteralPath $output -Recurse -Force
}

& cargo run --quiet --manifest-path (Join-Path $repoRoot "Cargo.toml") `
    -p dynwinrt-codegen -- generate `
    --winmd $aionWinmd `
    --ref $windowsWinmd `
    --namespace AionInstructPreview.Text `
    --class-name LanguageModel,LanguageModelContext,LanguageModelResponseResult `
    --output $output
if ($LASTEXITCODE -ne 0) {
    throw "Aion WinRT generation failed with exit code $LASTEXITCODE."
}

Write-Host "Generated Aion Instruct bindings from $aionWinmd"
