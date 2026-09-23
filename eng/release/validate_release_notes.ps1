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

function Get-Indent {
    param([string]$Line)

    return $Line.Length - $Line.TrimStart().Length
}

function Get-BlockEnd {
    param(
        [string[]]$Lines,
        [int]$Start,
        [int]$Indent,
        [switch]$AllowSequenceAtSameIndent
    )

    for ($index = $Start + 1; $index -lt $Lines.Count; $index++) {
        $trimmed = $Lines[$index].Trim()
        if (-not $trimmed -or $trimmed.StartsWith("#")) {
            continue
        }
        if ((Get-Indent $Lines[$index]) -le $Indent) {
            if ($AllowSequenceAtSameIndent -and
                (Get-Indent $Lines[$index]) -eq $Indent -and $trimmed.StartsWith("- ")) {
                continue
            }
            return $index
        }
    }

    return $Lines.Count
}

function Get-ChildBlockIndex {
    param(
        [string[]]$Lines,
        [int]$Start,
        [int]$End,
        [int]$ParentIndent,
        [string]$Key,
        [string]$Context
    )

    $found = @()
    for ($index = $Start + 1; $index -lt $End; $index++) {
        if ((Get-Indent $Lines[$index]) -eq ($ParentIndent + 2) -and
            $Lines[$index] -match "^\s*${Key}:\s*(?:#.*)?$") {
            $found += $index
        }
    }
    if ($found.Count -ne 1) {
        throw "$Context must have exactly one active '$Key' block"
    }
    return $found[0]
}

function ConvertFrom-YamlScalar {
    param([string]$Value)

    $valueWithoutComment = ($Value -split "\s+#", 2)[0].Trim()
    if ($valueWithoutComment -match "^'(?<value>(?:[^']|'')*)'$") {
        return $Matches.value.Replace("''", "'")
    }
    if ($valueWithoutComment -match '^"(?<value>[^"]*)"$') {
        return $Matches.value
    }
    return $valueWithoutComment
}

function Get-DirectMapping {
    param(
        [string[]]$Lines,
        [int]$Start,
        [int]$End,
        [int]$ParentIndent
    )

    $mapping = @{}
    $entryIndent = $ParentIndent + 2
    for ($index = $Start + 1; $index -lt $End; $index++) {
        $trimmed = $Lines[$index].Trim()
        if (-not $trimmed -or $trimmed.StartsWith("#")) {
            continue
        }
        if ((Get-Indent $Lines[$index]) -ne $entryIndent) {
            continue
        }
        if ($Lines[$index] -notmatch "^\s*(?<key>[A-Za-z][A-Za-z0-9]*):\s*(?<value>.*?)\s*$") {
            continue
        }

        $key = $Matches.key
        if ($mapping.ContainsKey($key)) {
            throw "Duplicate '$key' entry in active YAML mapping"
        }
        $mapping[$key] = ConvertFrom-YamlScalar $Matches.value
    }

    return $mapping
}

function Assert-TaskInput {
    param(
        [hashtable]$Inputs,
        [string]$Name,
        [string]$Expected
    )

    if (-not $Inputs.ContainsKey($Name)) {
        throw "GitHubRelease@1 input '$Name' is missing"
    }
    if ($Inputs[$Name] -cne $Expected) {
        throw "GitHubRelease@1 input '$Name' must be '$Expected', got '$($Inputs[$Name])'"
    }
}

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

$lines = @(Get-Content -LiteralPath $pipelinePath)
$yamlScalarPattern = "'(?:[^']|'')*'|`"[^`"]*`"|[^#\s]+"
$taskMatches = @()
for ($index = 0; $index -lt $lines.Count; $index++) {
    if ($lines[$index] -match "^(?<indent>\s*)-\s+task:\s*(?<value>$yamlScalarPattern)\s*(?:#.*)?$") {
        $taskIndent = $Matches.indent.Length
        $taskValue = ConvertFrom-YamlScalar $Matches.value
        if ($taskValue -cne "GitHubRelease@1") {
            continue
        }
        $taskMatches += [pscustomobject]@{
            Index = $index
            Indent = $taskIndent
        }
    }
}

if ($taskMatches.Count -ne 1) {
    throw "Expected exactly one active GitHubRelease@1 task, found $($taskMatches.Count)"
}

$taskIndex = $taskMatches[0].Index
$taskIndent = $taskMatches[0].Indent
$taskEnd = Get-BlockEnd -Lines $lines -Start $taskIndex -Indent $taskIndent

$inputsMatches = @()
for ($index = $taskIndex + 1; $index -lt $taskEnd; $index++) {
    if ((Get-Indent $lines[$index]) -eq ($taskIndent + 2) -and
        $lines[$index] -match "^\s*inputs:\s*(?:#.*)?$") {
        $inputsMatches += $index
    }
}
if ($inputsMatches.Count -ne 1) {
    throw "Expected exactly one active inputs block for GitHubRelease@1, found $($inputsMatches.Count)"
}

$inputsIndex = $inputsMatches[0]
$inputsIndent = Get-Indent $lines[$inputsIndex]
$inputsEnd = Get-BlockEnd -Lines $lines -Start $inputsIndex -Indent $inputsIndent
$inputs = Get-DirectMapping -Lines $lines -Start $inputsIndex -End $inputsEnd -ParentIndent $inputsIndent

Assert-TaskInput -Inputs $inputs -Name "releaseNotesSource" -Expected "filePath"
Assert-TaskInput -Inputs $inputs -Name "releaseNotesFilePath" `
    -Expected '$(Pipeline.Workspace)/release-notes/RELEASE_NOTES.md'
Assert-TaskInput -Inputs $inputs -Name "addChangeLog" -Expected "true"

$jobIndex = -1
$jobIndent = -1
for ($index = $taskIndex - 1; $index -ge 0; $index--) {
    if ($lines[$index] -match "^(?<indent>\s*)-\s+job:\s*(?<name>[^#\s]+)\s*(?:#.*)?$" -and
        $Matches.indent.Length -lt $taskIndent) {
        $jobIndex = $index
        $jobIndent = $Matches.indent.Length
        break
    }
}
if ($jobIndex -lt 0) {
    throw "GitHubRelease@1 is not inside an active job"
}

$jobEnd = Get-BlockEnd -Lines $lines -Start $jobIndex -Indent $jobIndent
if ($taskIndex -ge $jobEnd) {
    throw "GitHubRelease@1 is not inside the active job block"
}

$stepsMatches = @()
$templateContextMatches = @()
for ($index = $jobIndex + 1; $index -lt $jobEnd; $index++) {
    if ((Get-Indent $lines[$index]) -ne ($jobIndent + 2)) {
        continue
    }
    if ($lines[$index] -match "^\s*steps:\s*(?:#.*)?$") {
        $stepsMatches += $index
    }
    if ($lines[$index] -match "^\s*templateContext:\s*(?:#.*)?$") {
        $templateContextMatches += $index
    }
}
if ($stepsMatches.Count -ne 1) {
    throw "GitHubRelease@1 job must have exactly one active steps block"
}

$stepsIndex = $stepsMatches[0]
$stepsIndent = Get-Indent $lines[$stepsIndex]
$stepsEnd = Get-BlockEnd -Lines $lines -Start $stepsIndex -Indent $stepsIndent
if ($taskIndex -le $stepsIndex -or $taskIndex -ge $stepsEnd) {
    throw "GitHubRelease@1 must be inside the release job's active steps block"
}
if ($templateContextMatches.Count -ne 1) {
    throw "GitHubRelease@1 job must have exactly one active templateContext block"
}

$templateContextIndex = $templateContextMatches[0]
$templateContextIndent = Get-Indent $lines[$templateContextIndex]
$templateContextEnd = Get-BlockEnd -Lines $lines -Start $templateContextIndex `
    -Indent $templateContextIndent
$templateContext = Get-DirectMapping -Lines $lines -Start $templateContextIndex `
    -End $templateContextEnd -ParentIndent $templateContextIndent
if (-not $templateContext.ContainsKey("type") -or
    $templateContext["type"] -cne "releaseJob") {
    throw "GitHubRelease@1 job must use templateContext type 'releaseJob'"
}

$stepIndent = $stepsIndent + 2
for ($index = $stepsIndex + 1; $index -lt $stepsEnd; $index++) {
    if ((Get-Indent $lines[$index]) -eq $stepIndent -and
        $lines[$index] -match "^\s*-\s+checkout:\s*(?<value>$yamlScalarPattern)\s*(?:#.*)?$") {
        if ((ConvertFrom-YamlScalar $Matches.value) -cne "none") {
            throw "GitHubRelease@1 release job must not checkout source; use artifact job inputs"
        }
    }
}

$jobInputsIndex = Get-ChildBlockIndex -Lines $lines -Start $templateContextIndex `
    -End $templateContextEnd -ParentIndent $templateContextIndent -Key "inputs" `
    -Context "GitHubRelease@1 templateContext"
$jobInputsIndent = Get-Indent $lines[$jobInputsIndex]
$jobInputsEnd = Get-BlockEnd -Lines $lines -Start $jobInputsIndex -Indent $jobInputsIndent `
    -AllowSequenceAtSameIndent
$notesInputs = @()
for ($index = $jobInputsIndex + 1; $index -lt $jobInputsEnd; $index++) {
    $indent = Get-Indent $lines[$index]
    if ($indent -notin @($jobInputsIndent, ($jobInputsIndent + 2)) -or
        $lines[$index] -notmatch "^\s*-\s+input:\s*(?<value>$yamlScalarPattern)\s*(?:#.*)?$") {
        continue
    }
    $inputType = ConvertFrom-YamlScalar $Matches.value
    $entryEnd = Get-BlockEnd -Lines $lines -Start $index -Indent $indent
    $entry = Get-DirectMapping -Lines $lines -Start $index -End $entryEnd -ParentIndent $indent
    if ($entry["artifactName"] -ceq "release-notes") {
        if ($inputType -cne "pipelineArtifact" -or
            $entry["targetPath"] -cne '$(Pipeline.Workspace)/release-notes' -or
            $entry.ContainsKey("pipeline")) {
            throw "release-notes job input must use this run's pipelineArtifact at '`$(Pipeline.Workspace)/release-notes'"
        }
        $notesInputs += $index
    }
}
if ($notesInputs.Count -ne 1) {
    throw "GitHubRelease@1 job must declare exactly one release-notes pipelineArtifact input"
}

$buildStages = @()
for ($index = 0; $index -lt $lines.Count; $index++) {
    if ($lines[$index] -match "^\s*-\s+stage:\s*(?<value>$yamlScalarPattern)\s*(?:#.*)?$" -and
        (ConvertFrom-YamlScalar $Matches.value) -ceq "Build") {
        $buildStages += $index
    }
}
if ($buildStages.Count -ne 1) {
    throw "Release notes require exactly one Build stage"
}
$buildIndex = $buildStages[0]
$buildIndent = Get-Indent $lines[$buildIndex]
$buildEnd = Get-BlockEnd -Lines $lines -Start $buildIndex -Indent $buildIndent
$buildJobs = @()
for ($index = $buildIndex + 1; $index -lt $buildEnd; $index++) {
    if ((Get-Indent $lines[$index]) -eq ($buildIndent + 2) -and
        $lines[$index] -match "^\s*-\s+job:\s*(?<value>$yamlScalarPattern)\s*(?:#.*)?$" -and
        (ConvertFrom-YamlScalar $Matches.value) -ceq "Build") {
        $buildJobs += $index
    }
}
if ($buildJobs.Count -ne 1) {
    throw "Release notes require exactly one Build job in the Build stage"
}
$buildJobIndex = $buildJobs[0]
$buildJobIndent = Get-Indent $lines[$buildJobIndex]
$buildJobEnd = Get-BlockEnd -Lines $lines -Start $buildJobIndex -Indent $buildJobIndent
$buildStepsIndex = Get-ChildBlockIndex -Lines $lines -Start $buildJobIndex `
    -End $buildJobEnd -ParentIndent $buildJobIndent -Key "steps" -Context "Build job"
$buildStepsIndent = Get-Indent $lines[$buildStepsIndex]
$buildStepsEnd = Get-BlockEnd -Lines $lines -Start $buildStepsIndex -Indent $buildStepsIndent
$notesStagingDirectory = '$(Build.ArtifactStagingDirectory)/release-notes'
$stagers = @()
$publishers = @()
for ($index = $buildStepsIndex + 1; $index -lt $buildStepsEnd; $index++) {
    $indent = Get-Indent $lines[$index]
    if ($indent -ne ($buildStepsIndent + 2) -or
        $lines[$index] -notmatch "^\s*-\s+task:\s*(?<value>$yamlScalarPattern)\s*(?:#.*)?$") {
        continue
    }
    $taskType = ConvertFrom-YamlScalar $Matches.value
    if ($taskType -cnotin @("1ES.PublishPipelineArtifact@1", "CopyFiles@2")) {
        continue
    }
    $end = Get-BlockEnd -Lines $lines -Start $index -Indent $indent
    $publishInputsIndex = Get-ChildBlockIndex -Lines $lines -Start $index -End $end `
        -ParentIndent $indent -Key "inputs" -Context "Artifact publisher"
    $publishInputsIndent = Get-Indent $lines[$publishInputsIndex]
    $publishInputsEnd = Get-BlockEnd -Lines $lines -Start $publishInputsIndex -Indent $publishInputsIndent
    $publishInputs = Get-DirectMapping -Lines $lines -Start $publishInputsIndex `
        -End $publishInputsEnd -ParentIndent $publishInputsIndent
    if ($taskType -ceq "CopyFiles@2" -and $publishInputs["TargetFolder"] -ceq $notesStagingDirectory) {
        if ($publishInputs["SourceFolder"] -cne '$(Build.SourcesDirectory)/eng/release' -or
            $publishInputs["Contents"] -cne "RELEASE_NOTES.md" -or
            $publishInputs["CleanTargetFolder"] -cne "true") {
            throw "Build must stage only the checked-in RELEASE_NOTES.md into a clean release-notes directory"
        }
        $task = Get-DirectMapping -Lines $lines -Start $index -End $end -ParentIndent $indent
        if (($task.ContainsKey("enabled") -and $task["enabled"] -cne "true") -or
            ($task.ContainsKey("condition") -and $task["condition"] -cne "succeeded()") -or
            ($task.ContainsKey("continueOnError") -and $task["continueOnError"] -cne "false")) {
            throw "Build release-notes staging must not be disabled, conditionally skipped, or ignore errors"
        }
        $stagers += $index
    }
    if ($taskType -ceq "1ES.PublishPipelineArtifact@1" -and $publishInputs["artifactName"] -ceq "release-notes") {
        if ($publishInputs["targetPath"] -cne $notesStagingDirectory) {
            throw "Build must publish the whole release-notes staging directory to include the generated SBOM, not a single file"
        }
        $task = Get-DirectMapping -Lines $lines -Start $index -End $end -ParentIndent $indent
        if (($task.ContainsKey("enabled") -and $task["enabled"] -cne "true") -or
            ($task.ContainsKey("condition") -and $task["condition"] -cne "succeeded()")) {
            throw "Build release-notes publisher must not be disabled or conditionally skipped"
        }
        $publishers += $index
    }
}
if ($publishers.Count -ne 1) {
    throw "Build job must publish exactly one release-notes pipeline artifact"
}
if ($stagers.Count -ne 1 -or $stagers[0] -ge $publishers[0]) {
    throw "Build job must stage release notes exactly once before publishing the directory artifact"
}

$releaseStageIndex = -1
for ($index = $jobIndex - 1; $index -ge 0; $index--) {
    if ((Get-Indent $lines[$index]) -lt $jobIndent -and
        $lines[$index] -match "^\s*-\s+stage:\s*(?<value>$yamlScalarPattern)\s*(?:#.*)?$") {
        $releaseStageIndex = $index
        break
    }
}
if ($releaseStageIndex -lt 0) {
    throw "GitHubRelease@1 job must be in a stage that depends on Build"
}
$releaseStageIndent = Get-Indent $lines[$releaseStageIndex]
$releaseStageEnd = Get-BlockEnd -Lines $lines -Start $releaseStageIndex -Indent $releaseStageIndent
$dependsIndex = Get-ChildBlockIndex -Lines $lines -Start $releaseStageIndex -End $releaseStageEnd `
    -ParentIndent $releaseStageIndent -Key "dependsOn" -Context "GitHub release stage"
$dependsIndent = Get-Indent $lines[$dependsIndex]
$dependsEnd = Get-BlockEnd -Lines $lines -Start $dependsIndex -Indent $dependsIndent `
    -AllowSequenceAtSameIndent
$dependsOnBuild = $false
for ($index = $dependsIndex + 1; $index -lt $dependsEnd; $index++) {
    if ($lines[$index] -match "^\s*-\s+(?<value>$yamlScalarPattern)\s*(?:#.*)?$" -and
        (ConvertFrom-YamlScalar $Matches.value) -ceq "Build") {
        $dependsOnBuild = $true
    }
}
if (-not $dependsOnBuild) {
    throw "GitHub release stage must depend on Build to consume its release-notes artifact"
}

Write-Host "Release notes configuration is valid: $releaseNotesRelativePath is passed from Build via the release-notes artifact, without release-job source checkout"
