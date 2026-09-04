#!/usr/bin/env pwsh

param(
    [string]$Winmd,
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

if ($Winmd) {
    $Winmd = Resolve-CallerPath $Winmd
} else {
    $Winmd = Get-ChildItem `
        "C:\Program Files (x86)\Windows Kits\10\UnionMetadata" `
        -Filter Windows.winmd `
        -Recurse `
        -ErrorAction SilentlyContinue |
        Sort-Object FullName -Descending |
        Select-Object -First 1 -ExpandProperty FullName
}
if (-not $Winmd -or -not (Test-Path -LiteralPath $Winmd -PathType Leaf)) {
    throw "Windows.winmd was not found. Install a Windows 10/11 SDK or pass -Winmd."
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

& $codegenCommand generate `
    --winmd $Winmd `
    --class-name "Windows.Media.SpeechSynthesis.SpeechSynthesizer,Windows.Media.Playback.MediaPlayer" `
    --lang py `
    --output $output
if ($LASTEXITCODE -ne 0) {
    throw "dynwinrt-codegen failed with exit code $LASTEXITCODE"
}

Write-Host "Generated text-to-speech bindings in $output"
