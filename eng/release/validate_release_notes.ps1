# Copyright (c) Microsoft Corporation.
# Licensed under the MIT License.

[CmdletBinding()]
param(
    [string]$RepositoryRoot = (Resolve-Path (Join-Path $PSScriptRoot "..\..")).Path
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$releaseNotesRelativePath = "eng/release/RELEASE_NOTES.md"
$releaseNotesPath = Join-Path $RepositoryRoot $releaseNotesRelativePath
$pipelinePath = Join-Path $RepositoryRoot ".pipelines/release.yml"

if (-not (Test-Path -LiteralPath $releaseNotesPath -PathType Leaf)) {
    throw "Release notes file is missing: $releaseNotesRelativePath"
}

$releaseNotes = Get-Content -LiteralPath $releaseNotesPath -Raw
if ([string]::IsNullOrWhiteSpace($releaseNotes)) {
    throw "Release notes file is empty: $releaseNotesRelativePath"
}

$pipeline = Get-Content -LiteralPath $pipelinePath -Raw
$expectedSource = "releaseNotesSource: 'filePath'"
$expectedPath = 'releaseNotesFilePath: ''$(Build.SourcesDirectory)/eng/release/RELEASE_NOTES.md'''
$expectedChangeLog = "addChangeLog: true"

if (-not $pipeline.Contains($expectedSource)) {
    throw "GitHubRelease@1 must use $expectedSource"
}
if (-not $pipeline.Contains($expectedPath)) {
    throw "GitHubRelease@1 must reference $releaseNotesRelativePath from Build.SourcesDirectory"
}
if (-not $pipeline.Contains($expectedChangeLog)) {
    throw "GitHubRelease@1 must preserve $expectedChangeLog"
}

Write-Host "Release notes configuration is valid: $releaseNotesRelativePath"
