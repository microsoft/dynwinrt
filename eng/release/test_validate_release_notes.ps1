# Copyright (c) Microsoft Corporation.
# Licensed under the MIT License.

[CmdletBinding()]
param(
    [ValidateNotNullOrEmpty()][string]$RepositoryRoot
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

if (-not $PSBoundParameters.ContainsKey("RepositoryRoot")) {
    # Windows PowerShell 5.1 evaluates dot-sourced parameter defaults in the caller's scope.
    $RepositoryRoot = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot "..\..")).Path
}

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

function Assert-ValidationPasses {
    param(
        [string]$Name,
        [string]$Pipeline,
        [string]$Notes
    )

    Write-TestFile -Path $tempPipelinePath -Contents $Pipeline
    Write-TestFile -Path $tempNotesPath -Contents $Notes
    & $validatorPath -RepositoryRoot $tempRoot | Out-Null
    Write-Host "Accepted valid configuration: $Name"
}

& $validatorPath -RepositoryRoot $RepositoryRoot

$validPipeline = Get-Content -LiteralPath $pipelinePath -Raw
$validNotes = Get-Content -LiteralPath $releaseNotesPath -Raw
$newline = if ($validPipeline.Contains("`r`n")) { "`r`n" } else { "`n" }

New-Item -ItemType Directory -Force -Path `
    (Split-Path $tempPipelinePath -Parent), `
    (Split-Path $tempNotesPath -Parent) | Out-Null

try {
    $agentTemp = Join-Path $tempRoot "agent temp"
    New-Item -ItemType Directory -Path $agentTemp -Force | Out-Null
    $wrapperPath = Join-Path $agentTemp "validate wrapper.ps1"
    Write-TestFile -Path $tempPipelinePath -Contents $validPipeline
    Write-TestFile -Path $tempNotesPath -Contents $validNotes
    Write-TestFile -Path $wrapperPath -Contents @'
param([string]$Validator, [string]$ExpectedRoot, [string]$FixtureRoot)
$ErrorActionPreference = "Stop"
Set-Location -LiteralPath $PSScriptRoot
. $Validator
if ($RepositoryRoot -cne $ExpectedRoot) {
    throw "Dot-sourced validator resolved the caller's directory instead of its own repository"
}
. $Validator -RepositoryRoot $FixtureRoot
if ($RepositoryRoot -cne $FixtureRoot) {
    throw "Explicit RepositoryRoot was not preserved"
}
$missingRoot = Join-Path $PSScriptRoot "missing-repository"
try {
    . $Validator -RepositoryRoot $missingRoot
    throw "Expected missing release notes to fail"
} catch {
    if ($_.Exception.Message -notlike "Release notes file is missing:*") {
        throw
    }
}
Write-Output "SCRIPT_ROOT_PROBE_OK"
'@
    $shells = @((Get-Process -Id $PID).Path)
    if ($env:OS -eq "Windows_NT") {
        $shells += Join-Path $env:SystemRoot "System32\WindowsPowerShell\v1.0\powershell.exe"
    }
    $scriptRepositoryRoot = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot "..\..")).Path
    foreach ($shell in ($shells | Select-Object -Unique)) {
        $probeOutput = & $shell -NoLogo -NoProfile -NonInteractive -ExecutionPolicy Bypass `
            -File $wrapperPath -Validator $validatorPath -ExpectedRoot $scriptRepositoryRoot `
            -FixtureRoot $tempRoot
        if ($LASTEXITCODE -ne 0 -or $probeOutput -notcontains "SCRIPT_ROOT_PROBE_OK") {
            throw "Release validator dot-source regression failed under ${shell}: $probeOutput"
        }
        Write-Host "Accepted agent temporary-wrapper invocation: $shell"
    }

    $releaseTask = "          - task: GitHubRelease@1"
    $notesInput = @(
        "          - input: pipelineArtifact"
        "            artifactName: release-notes"
        '            targetPath: $(Pipeline.Workspace)/release-notes'
    ) -join $newline
    $notesStager = @(
        "          - task: CopyFiles@2"
        "            displayName: Stage release notes"
        "            inputs:"
        '              SourceFolder: ''$(Build.SourcesDirectory)/eng/release'''
        "              Contents: RELEASE_NOTES.md"
        '              TargetFolder: ''$(Build.ArtifactStagingDirectory)/release-notes'''
        "              CleanTargetFolder: true"
    ) -join $newline
    $notesPublisher = @(
        "          - task: 1ES.PublishPipelineArtifact@1"
        "            displayName: Upload release notes"
        "            inputs:"
        '              targetPath: ''$(Build.ArtifactStagingDirectory)/release-notes'''
        "              artifactName: release-notes"
    ) -join $newline

    $singleTaskDoubleArtifact = $validPipeline.Replace(
        "          - task: GitHubRelease@1",
        "          - task: 'GitHubRelease@1'"
    ).Replace(
        "artifactName: release-notes",
        'artifactName: "release-notes"'
    )
    Assert-ValidationPasses -Name "single-quoted task and double-quoted artifact" `
        -Pipeline $singleTaskDoubleArtifact -Notes $validNotes

    $doubleTaskSingleArtifact = $validPipeline.Replace(
        "          - task: GitHubRelease@1",
        "          - task: `"GitHubRelease@1`""
    ).Replace(
        "artifactName: release-notes",
        "artifactName: 'release-notes'"
    )
    Assert-ValidationPasses -Name "double-quoted task and single-quoted artifact" `
        -Pipeline $doubleTaskSingleArtifact -Notes $validNotes

    $explicitNoCheckout = $validPipeline.Replace(
        $releaseTask, "          - checkout: none${newline}${releaseTask}"
    )
    Assert-ValidationPasses -Name "release job explicitly disables checkout" `
        -Pipeline $explicitNoCheckout -Notes $validNotes

    $commentedCheckout = $validPipeline.Replace(
        $releaseTask, "          # - checkout: self${newline}${releaseTask}"
    )
    Assert-ValidationPasses -Name "commented source checkout is inactive" `
        -Pipeline $commentedCheckout -Notes $validNotes

    $quotedDuplicateTasks = @(
        "          - task: 'GitHubRelease@1'"
        ""
        "          - task: `"GitHubRelease@1`""
    ) -join $newline
    $duplicateTaskPipeline = $validPipeline.Replace(
        "          - task: GitHubRelease@1",
        $quotedDuplicateTasks
    )
    Assert-ValidationFails -Name "quoted duplicate GitHub release task" `
        -Pipeline $duplicateTaskPipeline -Notes $validNotes -CreateNotes $true `
        -ExpectedMessage "Expected exactly one active GitHubRelease@1 task, found 2"

    $commentedExpectedValues = @(
        "              releaseNotesSource: 'inline'"
        "              # releaseNotesSource: 'filePath'"
        '              # releaseNotesFilePath: ''$(Pipeline.Workspace)/release-notes/RELEASE_NOTES.md'''
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
        '              releaseNotesFilePath: ''$(Pipeline.Workspace)/release-notes/RELEASE_NOTES.md'''
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
        'releaseNotesFilePath: ''$(Pipeline.Workspace)/release-notes/RELEASE_NOTES.md''',
        'releaseNotesFilePath: ''$(Pipeline.Workspace)/release-notes/BROKEN.md'''
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

    foreach ($checkout in @("self", "'self'", '"self"', "anotherRepository")) {
        $checkoutPipeline = $validPipeline.Replace(
            $releaseTask, "          - checkout: ${checkout}${newline}${releaseTask}"
        )
        Assert-ValidationFails -Name "release job source checkout $checkout" `
            -Pipeline $checkoutPipeline -Notes $validNotes -CreateNotes $true `
            -ExpectedMessage "*must not checkout source*"
    }
    $lateCheckoutPipeline = $validPipeline.Replace(
        "              addChangeLog: true",
        "              addChangeLog: true${newline}          - checkout: self"
    )
    Assert-ValidationFails -Name "source checkout after release task" `
        -Pipeline $lateCheckoutPipeline -Notes $validNotes -CreateNotes $true `
        -ExpectedMessage "*must not checkout source*"

    $missingArtifact = $validPipeline.Replace($notesInput, "          # release-notes input removed")
    Assert-ValidationFails -Name "missing release notes job input" `
        -Pipeline $missingArtifact -Notes $validNotes -CreateNotes $true `
        -ExpectedMessage "*exactly one release-notes pipelineArtifact input*"
    $duplicateArtifact = $validPipeline.Replace($notesInput, "${notesInput}${newline}${notesInput}")
    Assert-ValidationFails -Name "duplicate release notes job input" `
        -Pipeline $duplicateArtifact -Notes $validNotes -CreateNotes $true `
        -ExpectedMessage "*exactly one release-notes pipelineArtifact input*"
    foreach ($replacement in @(
        $notesInput.Replace("input: pipelineArtifact", "input: otherArtifact"),
        $notesInput.Replace('$(Pipeline.Workspace)/release-notes', '$(Pipeline.Workspace)/wrong-notes'),
        "${notesInput}${newline}            pipeline: anotherPipeline"
    )) {
        Assert-ValidationFails -Name "incorrect release notes job input" `
            -Pipeline $validPipeline.Replace($notesInput, $replacement) `
            -Notes $validNotes -CreateNotes $true `
            -ExpectedMessage "*job input must use this run's pipelineArtifact*"
    }

    $taskArtifact = (($notesInput -split "\r?\n" | ForEach-Object { "    $_" }) -join $newline)
    $misplacedArtifact = $missingArtifact.Replace(
        $releaseTask,
        "          - task: PowerShell@2${newline}            inputs:${newline}${taskArtifact}${newline}${releaseTask}"
    )
    Assert-ValidationFails -Name "artifact input outside templateContext" `
        -Pipeline $misplacedArtifact -Notes $validNotes -CreateNotes $true `
        -ExpectedMessage "*exactly one release-notes pipelineArtifact input*"

    $missingPublisher = $validPipeline.Replace($notesPublisher, "          # release-notes publisher removed")
    Assert-ValidationFails -Name "missing Build artifact producer" `
        -Pipeline $missingPublisher -Notes $validNotes -CreateNotes $true `
        -ExpectedMessage "*Build job must publish exactly one release-notes*"
    $duplicatePublisher = $validPipeline.Replace($notesPublisher, "${notesPublisher}${newline}${notesPublisher}")
    Assert-ValidationFails -Name "duplicate Build artifact producer" `
        -Pipeline $duplicatePublisher -Notes $validNotes -CreateNotes $true `
        -ExpectedMessage "*Build job must publish exactly one release-notes*"
    $singleFilePublisher = $validPipeline.Replace(
        $notesPublisher,
        $notesPublisher.Replace(
            '$(Build.ArtifactStagingDirectory)/release-notes',
            '$(Build.SourcesDirectory)/eng/release/RELEASE_NOTES.md'
        )
    )
    Assert-ValidationFails -Name "single-file artifact omits sibling SBOM" `
        -Pipeline $singleFilePublisher -Notes $validNotes -CreateNotes $true `
        -ExpectedMessage "*whole release-notes staging directory*"
    $nestedFilePublisher = $validPipeline.Replace(
        $notesPublisher,
        $notesPublisher.Replace(
            '$(Build.ArtifactStagingDirectory)/release-notes',
            '$(Build.ArtifactStagingDirectory)/release-notes/RELEASE_NOTES.md'
        )
    )
    Assert-ValidationFails -Name "staged single-file artifact omits sibling SBOM" `
        -Pipeline $nestedFilePublisher -Notes $validNotes -CreateNotes $true `
        -ExpectedMessage "*whole release-notes staging directory*"
    $missingStager = $validPipeline.Replace($notesStager, "          # release-notes staging removed")
    Assert-ValidationFails -Name "missing notes staging step" `
        -Pipeline $missingStager -Notes $validNotes -CreateNotes $true `
        -ExpectedMessage "*stage release notes exactly once before publishing*"
    $duplicateStager = $validPipeline.Replace($notesStager, "${notesStager}${newline}${notesStager}")
    Assert-ValidationFails -Name "duplicate notes staging step" `
        -Pipeline $duplicateStager -Notes $validNotes -CreateNotes $true `
        -ExpectedMessage "*stage release notes exactly once before publishing*"
    $lateStager = $missingStager.Replace($notesPublisher, "${notesPublisher}${newline}${notesStager}")
    Assert-ValidationFails -Name "notes staged after artifact publication" `
        -Pipeline $lateStager -Notes $validNotes -CreateNotes $true `
        -ExpectedMessage "*stage release notes exactly once before publishing*"
    foreach ($replacement in @(
        $notesStager.Replace("Contents: RELEASE_NOTES.md", "Contents: README.md"),
        $notesStager.Replace("Contents: RELEASE_NOTES.md", "Contents: '**'"),
        $notesStager.Replace('$(Build.SourcesDirectory)/eng/release', '$(Build.SourcesDirectory)/unrelated'),
        $notesStager.Replace("CleanTargetFolder: true", "CleanTargetFolder: false")
    )) {
        Assert-ValidationFails -Name "incorrect notes staging source or cleanup" `
            -Pipeline $validPipeline.Replace($notesStager, $replacement) `
            -Notes $validNotes -CreateNotes $true `
            -ExpectedMessage "*stage only the checked-in RELEASE_NOTES.md into a clean*"
    }
    $wrongStagingDirectory = $validPipeline.Replace(
        $notesStager,
        $notesStager.Replace('$(Build.ArtifactStagingDirectory)/release-notes', '$(Build.ArtifactStagingDirectory)/wrong-notes')
    )
    Assert-ValidationFails -Name "notes staged outside the published directory" `
        -Pipeline $wrongStagingDirectory -Notes $validNotes -CreateNotes $true `
        -ExpectedMessage "*stage release notes exactly once before publishing*"
    foreach ($disabled in @("enabled: false", "condition: false", "continueOnError: true")) {
        $disabledStager = $validPipeline.Replace(
            $notesStager,
            $notesStager.Replace("            displayName: Stage release notes", "            ${disabled}${newline}            displayName: Stage release notes")
        )
        Assert-ValidationFails -Name "disabled notes staging $disabled" `
            -Pipeline $disabledStager -Notes $validNotes -CreateNotes $true `
            -ExpectedMessage "*staging must not be disabled*"
    }
    foreach ($disabled in @("enabled: false", "condition: false")) {
        $disabledPublisher = $validPipeline.Replace(
            $notesPublisher,
            $notesPublisher.Replace("            displayName: Upload release notes", "            ${disabled}${newline}            displayName: Upload release notes")
        )
        Assert-ValidationFails -Name "disabled notes publisher $disabled" `
            -Pipeline $disabledPublisher -Notes $validNotes -CreateNotes $true `
            -ExpectedMessage "*publisher must not be disabled*"
    }
    $wrongProducerJob = $missingPublisher.Replace($releaseTask, "${notesPublisher}${newline}${releaseTask}")
    Assert-ValidationFails -Name "artifact published in release job instead of Build" `
        -Pipeline $wrongProducerJob -Notes $validNotes -CreateNotes $true `
        -ExpectedMessage "*Build job must publish exactly one release-notes*"
    $missingBuildDependency = $validPipeline.Replace(
        "        - Build${newline}        - Wait_Python",
        "        - Wait_Python"
    )
    Assert-ValidationFails -Name "release stage missing Build dependency" `
        -Pipeline $missingBuildDependency -Notes $validNotes -CreateNotes $true `
        -ExpectedMessage "*must depend on Build*"

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
