# Copyright (c) Microsoft Corporation.
# Licensed under the MIT License.

[CmdletBinding()]
param(
    [Parameter(Mandatory)][ValidatePattern('^[0-9][0-9A-Za-z.+-]*$')][string]$Version,
    [Parameter(Mandatory)][ValidatePattern('^[0-9a-fA-F]{40}$')][string]$SourceSha,
    [Parameter(Mandatory)][ValidateSet('assemble-release', 'github-release')][string]$JobName,
    [ValidateRange(1, [long]::MaxValue)][long]$RunId,
    [switch]$ExportRunId,
    [ValidateRange(1, 120)][int]$DiscoveryAttempts = 10,
    [ValidateRange(1, 120)][int]$WaitAttempts = 120,
    [ValidateRange(0, 60)][int]$PollSeconds = 30,
    [scriptblock]$ApiRequest,
    [scriptblock]$Sleep
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

if (-not $ApiRequest) {
    $ApiRequest = {
        param($Uri)
        Invoke-RestMethod -Uri $Uri -TimeoutSec 30 -Headers @{
            Accept = 'application/vnd.github+json'
            'User-Agent' = 'microsoft-dynwinrt-ado-release'
            'X-GitHub-Api-Version' = '2022-11-28'
        }
    }
}
if (-not $Sleep) {
    $Sleep = { param($Seconds) Start-Sleep -Seconds $Seconds }
}

$apiRoot = 'https://api.github.com/repos/microsoft/dynwinrt'
$workflowPath = '.github/workflows/python-release.yml'
$tag = "v$Version"

function Invoke-GitHubApi {
    param([string]$Uri)

    for ($requestAttempt = 1; $requestAttempt -le 3; $requestAttempt++) {
        try {
            return (& $ApiRequest $Uri)
        } catch {
            if ($requestAttempt -eq 3) {
                throw "GitHub API request failed for '${Uri}': $($_.Exception.Message)"
            }
            Write-Warning "GitHub API request failed; retrying: $($_.Exception.Message)"
            & $Sleep (10 * $requestAttempt)
        }
    }
}

function Test-RunIdentity {
    param($Run)

    return ($Run.path -ceq $workflowPath -and
        $Run.event -ceq 'push' -and
        $Run.head_branch -ceq $tag -and
        $Run.head_sha -eq $SourceSha)
}

if (-not $PSBoundParameters.ContainsKey('RunId')) {
    for ($attempt = 1; $attempt -le $DiscoveryAttempts; $attempt++) {
        $matchingRuns = @(
            $page = 1
            do {
                $response = Invoke-GitHubApi "$apiRoot/actions/workflows/python-release.yml/runs?event=push&head_sha=$SourceSha&per_page=100&page=$page"
                $response.workflow_runs | Where-Object { Test-RunIdentity $_ }
                $page++
            } while (@($response.workflow_runs).Count -eq 100)
        )
        if ($matchingRuns.Count -gt 0) {
            $selected = $matchingRuns | Sort-Object `
                @{ Expression = { [datetimeoffset]$_.created_at }; Descending = $true }, `
                @{ Expression = { [long]$_.id }; Descending = $true } |
                Select-Object -First 1
            $RunId = [long]$selected.id
            if ($matchingRuns.Count -gt 1) {
                Write-Warning "Found $($matchingRuns.Count) Python workflows for $tag at $SourceSha; choosing newest run $RunId (created_at, then id)."
            }
            break
        }
        if ($attempt -lt $DiscoveryAttempts) {
            Write-Host "Waiting for the Python tag workflow (attempt $attempt/$DiscoveryAttempts)..."
            & $Sleep $PollSeconds
        }
    }
    if (-not $RunId) {
        throw "Python tag workflow for $tag at $SourceSha was not found after $DiscoveryAttempts discovery attempts."
    }
}

Write-Host "Waiting for Python run $RunId, job $JobName ($tag at $SourceSha)."
for ($attempt = 1; $attempt -le $WaitAttempts; $attempt++) {
    $run = Invoke-GitHubApi "$apiRoot/actions/runs/$RunId"
    if ([long]$run.id -ne $RunId -or -not (Test-RunIdentity $run)) {
        throw "Python run $RunId does not match $workflowPath, push event, tag $tag and source SHA $SourceSha."
    }

    $matchingJobs = @(
        $page = 1
        do {
            $response = Invoke-GitHubApi "$apiRoot/actions/runs/$RunId/jobs?filter=latest&per_page=100&page=$page"
            $response.jobs | Where-Object name -CEQ $JobName
            $page++
        } while (@($response.jobs).Count -eq 100)
    )
    if ($matchingJobs.Count -gt 1) {
        throw "Python run $RunId has multiple '$JobName' jobs in its latest attempt."
    }
    if ($matchingJobs.Count -eq 1 -and $matchingJobs[0].status -eq 'completed') {
        if ($matchingJobs[0].conclusion -ne 'success') {
            throw "Python run $RunId job '$JobName' concluded '$($matchingJobs[0].conclusion)'."
        }
        if ($JobName -eq 'github-release') {
            # Keep the release target guard, but never use embedded assets as an upload signal.
            $release = Invoke-GitHubApi "$apiRoot/releases/tags/$tag"
            if ($release.tag_name -cne $tag -or $release.target_commitish -ne $SourceSha) {
                throw "GitHub release for $tag does not match the exact tag and source SHA $SourceSha."
            }
        }
        Write-Host "Python run $RunId job '$JobName' succeeded."
        if ($ExportRunId) {
            Write-Host "##vso[task.setvariable variable=pythonRunId;isOutput=true]$RunId"
        }
        return $RunId
    }
    if ($run.status -eq 'completed') {
        throw "Python run $RunId concluded '$($run.conclusion)' without a successful '$JobName' job."
    }
    if ($attempt -lt $WaitAttempts) {
        Write-Host "Waiting for Python job '$JobName' (attempt $attempt/$WaitAttempts)..."
        & $Sleep $PollSeconds
    }
}
throw "Python run $RunId job '$JobName' did not succeed after $WaitAttempts polling attempts."
