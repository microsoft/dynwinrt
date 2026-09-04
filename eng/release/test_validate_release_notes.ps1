# Copyright (c) Microsoft Corporation.
# Licensed under the MIT License.

[CmdletBinding()]
param(
    [string]$RepositoryRoot = (Resolve-Path (Join-Path $PSScriptRoot "..\..")).Path
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$validatorPath = Join-Path $PSScriptRoot "validate_release_notes.ps1"
$pipelinePath = Join-Path $RepositoryRoot ".pipelines/release.yml"
$releaseNotesPath = Join-Path $RepositoryRoot "eng/release/RELEASE_NOTES.md"
$tempRoot = Join-Path ([IO.Path]::GetTempPath()) "dynwinrt-release-notes-$([guid]::NewGuid())"
$tempPipelinePath = Join-Path $tempRoot ".pipelines/release.yml"
$tempNotesPath = Join-Path $tempRoot "eng/release/RELEASE_NOTES.md"
$utf8NoBom = [Text.UTF8Encoding]::new($false)

function Write-TestFile {
    param(
        [string]$Path,
        [string]$Contents
    )

    [IO.File]::WriteAllText($Path, $Contents, $utf8NoBom)
}

function Assert-ValidationFails {
    param(
        [string]$Name,
        [string]$Pipeline,
        [AllowEmptyString()][string]$Notes,
        [bool]$CreateNotes,
        [string]$ExpectedMessage
    )

    Write-TestFile -Path $tempPipelinePath -Contents $Pipeline
    if ($CreateNotes) {
        Write-TestFile -Path $tempNotesPath -Contents $Notes
    } else {
        Remove-Item -LiteralPath $tempNotesPath -Force -ErrorAction SilentlyContinue
    }

    try {
        & $validatorPath -RepositoryRoot $tempRoot | Out-Null
        throw "Expected validation failure for '$Name'"
    } catch {
        if ($_.Exception.Message -notlike $ExpectedMessage) {
            throw "Unexpected failure for '$Name': $($_.Exception.Message)"
        }
    }

    Write-Host "Rejected invalid configuration: $Name"
}

& $validatorPath -RepositoryRoot $RepositoryRoot

$validPipeline = Get-Content -LiteralPath $pipelinePath -Raw
$validNotes = Get-Content -LiteralPath $releaseNotesPath -Raw
$newline = if ($validPipeline.Contains("`r`n")) { "`r`n" } else { "`n" }

New-Item -ItemType Directory -Force -Path `
    (Split-Path $tempPipelinePath -Parent), `
    (Split-Path $tempNotesPath -Parent) | Out-Null

try {
    $commentedExpectedValues = @(
        "              releaseNotesSource: 'inline'"
        "              # releaseNotesSource: 'filePath'"
        '              # releaseNotesFilePath: ''$(Build.SourcesDirectory)/eng/release/RELEASE_NOTES.md'''
        "              # addChangeLog: true"
    ) -join $newline
    $commentedPipeline = $validPipeline.Replace(
        "              releaseNotesSource: 'filePath'",
        $commentedExpectedValues
    )
    Assert-ValidationFails -Name "expected values only in comments" `
        -Pipeline $commentedPipeline -Notes $validNotes -CreateNotes $true `
        -ExpectedMessage "*releaseNotesSource*must be 'filePath'*"

    $activeInlinePipeline = $validPipeline.Replace(
        "              releaseNotesSource: 'filePath'",
        "              releaseNotesSource: 'inline'"
    )
    $unrelatedExpectedValues = @(
        "          - task: PowerShell@2"
        "            displayName: Unrelated expected values"
        "            inputs:"
        "              targetType: inline"
        "              script: Write-Host test"
        "            env:"
        "              releaseNotesSource: 'filePath'"
        '              releaseNotesFilePath: ''$(Build.SourcesDirectory)/eng/release/RELEASE_NOTES.md'''
        "              addChangeLog: true"
        ""
        "          - task: GitHubRelease@1"
    ) -join $newline
    $unrelatedPipeline = $activeInlinePipeline.Replace(
        "          - task: GitHubRelease@1",
        $unrelatedExpectedValues
    )
    Assert-ValidationFails -Name "expected values on an unrelated task" `
        -Pipeline $unrelatedPipeline -Notes $validNotes -CreateNotes $true `
        -ExpectedMessage "*releaseNotesSource*must be 'filePath'*"

    $wrongPathPipeline = $validPipeline.Replace(
        'releaseNotesFilePath: ''$(Build.SourcesDirectory)/eng/release/RELEASE_NOTES.md''',
        'releaseNotesFilePath: ''$(Build.SourcesDirectory)/eng/release/BROKEN.md'''
    )
    Assert-ValidationFails -Name "wrong active release notes path" `
        -Pipeline $wrongPathPipeline -Notes $validNotes -CreateNotes $true `
        -ExpectedMessage "*releaseNotesFilePath*RELEASE_NOTES.md*BROKEN.md*"

    $disabledChangeLogPipeline = $validPipeline.Replace(
        "addChangeLog: true",
        "addChangeLog: false"
    )
    Assert-ValidationFails -Name "disabled active changelog" `
        -Pipeline $disabledChangeLogPipeline -Notes $validNotes -CreateNotes $true `
        -ExpectedMessage "*addChangeLog*must be 'true'*"

    $checkoutBlock = "          - checkout: self${newline}            fetchDepth: 1"
    $checkoutNoneBlock = "          - checkout: none${newline}          # - checkout: self${newline}          #   fetchDepth: 1"
    $missingCheckoutPipeline = $validPipeline.Replace($checkoutBlock, $checkoutNoneBlock)
    Assert-ValidationFails -Name "release job source checkout unavailable" `
        -Pipeline $missingCheckoutPipeline -Notes $validNotes -CreateNotes $true `
        -ExpectedMessage "*explicitly checkout 'self'*"

    Assert-ValidationFails -Name "empty release notes" `
        -Pipeline $validPipeline -Notes "" -CreateNotes $true `
        -ExpectedMessage "Release notes file is empty:*"

    Assert-ValidationFails -Name "missing release notes" `
        -Pipeline $validPipeline -Notes "" -CreateNotes $false `
        -ExpectedMessage "Release notes file is missing:*"
} finally {
    Remove-Item -LiteralPath $tempRoot -Recurse -Force -ErrorAction SilentlyContinue
}

Write-Host "Release notes validation regression tests passed"
