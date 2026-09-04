# Copyright (c) Microsoft Corporation.
# Licensed under the MIT License.

[CmdletBinding()]
param(
    [string]$RepositoryRoot = (Resolve-Path (Join-Path $PSScriptRoot "..\..")).Path
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

function Get-Indent {
    param([string]$Line)

    return $Line.Length - $Line.TrimStart().Length
}

function Get-BlockEnd {
    param(
        [string[]]$Lines,
        [int]$Start,
        [int]$Indent
    )

    for ($index = $Start + 1; $index -lt $Lines.Count; $index++) {
        $trimmed = $Lines[$index].Trim()
        if (-not $trimmed -or $trimmed.StartsWith("#")) {
            continue
        }
        if ((Get-Indent $Lines[$index]) -le $Indent) {
            return $index
        }
    }

    return $Lines.Count
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
$taskMatches = @()
for ($index = 0; $index -lt $lines.Count; $index++) {
    if ($lines[$index] -match "^(?<indent>\s*)-\s+task:\s*GitHubRelease@1\s*(?:#.*)?$") {
        $taskMatches += [pscustomobject]@{
            Index = $index
            Indent = $Matches.indent.Length
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
    -Expected '$(Build.SourcesDirectory)/eng/release/RELEASE_NOTES.md'
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
$checkoutMatches = @()
for ($index = $stepsIndex + 1; $index -lt $taskIndex; $index++) {
    if ((Get-Indent $lines[$index]) -eq $stepIndent -and
        $lines[$index] -match "^\s*-\s+checkout:\s*(?<value>[^#\s]+)\s*(?:#.*)?$") {
        $checkoutMatches += [pscustomobject]@{
            Index = $index
            Value = $Matches.value
        }
    }
}
if ($checkoutMatches.Count -ne 1 -or $checkoutMatches[0].Value -cne "self") {
    throw "GitHubRelease@1 release job must explicitly checkout 'self' before the task"
}

$checkoutIndex = $checkoutMatches[0].Index
$checkoutEnd = Get-BlockEnd -Lines $lines -Start $checkoutIndex -Indent $stepIndent
$checkout = Get-DirectMapping -Lines $lines -Start $checkoutIndex -End $checkoutEnd `
    -ParentIndent $stepIndent
if (-not $checkout.ContainsKey("fetchDepth") -or $checkout["fetchDepth"] -cne "1") {
    throw "GitHubRelease@1 release job checkout must use fetchDepth '1'"
}

Write-Host "Release notes configuration is valid: $releaseNotesRelativePath is available from the release job checkout"
