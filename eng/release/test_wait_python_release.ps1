# Copyright (c) Microsoft Corporation.
# Licensed under the MIT License.

[CmdletBinding()]
param()

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$waitScript = Join-Path $PSScriptRoot 'wait_python_release.ps1'
$repositoryRoot = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..\..')).Path
$sha = 'a' * 40
$version = '0.1.0-preview.22'
$tag = "v$version"
$apiRoot = 'https://api.github.com/repos/microsoft/dynwinrt'
$discoveryPath = "/actions/workflows/python-release.yml/runs?event=push&head_sha=$sha&per_page=100&page=1"
$jobsPath = '/actions/runs/42/jobs?filter=latest&per_page=100&page=1'
$caseCount = 0

function New-Run {
    param(
        [long]$Id = 42,
        [string]$Created = '2026-09-23T10:00:00Z',
        [string]$Status = 'in_progress',
        [AllowNull()][string]$Conclusion = $null,
        [string]$TagName = $tag,
        [string]$ShaValue = $sha,
        [string]$EventName = 'push',
        [string]$Path = '.github/workflows/python-release.yml'
    )
    return [pscustomobject]@{
        id = $Id; created_at = $Created; status = $Status; conclusion = $Conclusion
        head_branch = $TagName; head_sha = $ShaValue; event = $EventName; path = $Path
    }
}

function New-Job {
    param(
        [string]$Name = 'assemble-release',
        [string]$Status = 'completed',
        [AllowNull()][string]$Conclusion = 'success'
    )
    return [pscustomobject]@{ name = $Name; status = $Status; conclusion = $Conclusion }
}

function New-Response {
    param([string]$Path, $Body, [string]$ErrorMessage = '')
    return [pscustomobject]@{ Path = $Path; Body = $Body; ErrorMessage = $ErrorMessage }
}

function Assert-True {
    param([bool]$Condition, [string]$Message)
    if (-not $Condition) { throw $Message }
}

function Test-Wait {
    param(
        [string]$Name,
        [object[]]$Responses,
        [hashtable]$Arguments = @{},
        [string]$ExpectedError = '',
        [long]$ExpectedId = 42,
        [string]$ExpectedWarning = '',
        [int[]]$ExpectedSleeps = @(),
        [switch]$DotSource,
        [switch]$ProductionDefaults
    )

    $state = @{
        Steps = [System.Collections.Generic.Queue[object]]::new()
        Sleeps = [System.Collections.Generic.List[int]]::new()
        Output = [System.Collections.Generic.List[object]]::new()
    }
    foreach ($response in $Responses) { $state.Steps.Enqueue($response) }
    $request = {
        param($Uri)
        if ($state.Steps.Count -eq 0) { throw "Unexpected request: $Uri" }
        $step = $state.Steps.Dequeue()
        if ($Uri -cne "$apiRoot$($step.Path)") {
            throw "Expected API path '$($step.Path)', got '$Uri'"
        }
        if ($step.ErrorMessage) { throw $step.ErrorMessage }
        return $step.Body
    }.GetNewClosure()
    $sleeper = { param($Seconds) $state.Sleeps.Add($Seconds) }.GetNewClosure()
    $parameters = @{
        Version = $version; SourceSha = $sha; JobName = 'assemble-release'
        ApiRequest = $request; Sleep = $sleeper
        WarningVariable = 'warnings'; WarningAction = 'SilentlyContinue'
    }
    if (-not $ProductionDefaults) {
        $parameters.DiscoveryAttempts = 2
        $parameters.WaitAttempts = 2
        $parameters.PollSeconds = 30
    }
    foreach ($key in $Arguments.Keys) { $parameters[$key] = $Arguments[$key] }
    $warnings = @()
    $caught = ''
    try {
        if ($DotSource) {
            . $waitScript @parameters 6>&1 | ForEach-Object { $state.Output.Add($_) }
        } else {
            & $waitScript @parameters 6>&1 | ForEach-Object { $state.Output.Add($_) }
        }
    } catch {
        $caught = $_.Exception.Message
    }
    if ($ExpectedError) {
        Assert-True ($caught -like $ExpectedError) "$Name expected '$ExpectedError', got '$caught'"
        Assert-True (-not ($state.Output -match '##vso')) "$Name exported a run ID after failure"
    } else {
        Assert-True (-not $caught) "$Name failed: $caught"
        $ids = @($state.Output | Where-Object { $_ -is [long] })
        Assert-True ($ids.Count -eq 1 -and $ids[0] -eq $ExpectedId) "$Name returned the wrong run ID"
    }
    Assert-True ($state.Steps.Count -eq 0) "$Name did not consume all expected requests"
    Assert-True (($state.Sleeps -join ',') -ceq ($ExpectedSleeps -join ',')) "$Name used unexpected sleeps: $($state.Sleeps -join ',')"
    if ($ExpectedWarning) {
        Assert-True (($warnings -join "`n") -like $ExpectedWarning) "$Name did not emit the expected warning"
    } else {
        Assert-True (@($warnings).Count -eq 0) "$Name emitted unexpected warnings: $warnings"
    }
    if (-not $ExpectedError -and $Arguments.ContainsKey('ExportRunId')) {
        Assert-True (($state.Output | ForEach-Object { "$_" }) -contains
            "##vso[task.setvariable variable=pythonRunId;isOutput=true]$ExpectedId") "$Name did not export the Azure output"
    }
    $script:caseCount++
    Write-Host "PASS: $Name"
}

$run = New-Run
$assembly = @{ jobs = @(New-Job) }
$upload = @{ jobs = @(New-Job -Name 'github-release') }
$release = @{ tag_name = $tag; target_commitish = $sha; assets = @() }

Test-Wait 'production discovery defaults: 10 attempts, 30-second interval' -ProductionDefaults -Responses @(
    1..10 | ForEach-Object { New-Response $discoveryPath @{ workflow_runs = @() } }
) -ExpectedSleeps (@(30) * 9) -ExpectedError '*not found after 10 discovery attempts (30s interval).'

foreach ($jobName in @('assemble-release', 'github-release')) {
    Test-Wait "production $jobName defaults: 40 requests, 57 minutes of sleeps" -ProductionDefaults `
        -Arguments @{ RunId = 42; JobName = $jobName } -Responses @(
            1..20 | ForEach-Object {
                New-Response '/actions/runs/42' $run
                New-Response $jobsPath @{ jobs = @() }
            }
        ) -ExpectedSleeps (@(180) * 19) -ExpectedError "*job '$jobName' did not succeed after 20 polling attempts (180s interval)."
}

Test-Wait 'discovery interval is independent of job interval' -Arguments @{ DiscoveryPollSeconds = 7; PollSeconds = 300 } -Responses @(
    (New-Response $discoveryPath @{ workflow_runs = @() })
    (New-Response $discoveryPath @{ workflow_runs = @($run) })
    (New-Response '/actions/runs/42' $run)
    (New-Response $jobsPath @{ jobs = @() })
    (New-Response '/actions/runs/42' $run)
    (New-Response $jobsPath $assembly)
) -ExpectedSleeps @(7, 300)

Test-Wait 'exact workflow, event, tag and SHA filtering' -Arguments @{ ExportRunId = $true } -Responses @(
    (New-Response $discoveryPath @{ workflow_runs = @(
        (New-Run -Id 50 -TagName 'v0.1.0-preview.21')
        (New-Run -Id 51 -ShaValue ('b' * 40))
        (New-Run -Id 52 -EventName 'workflow_dispatch')
        (New-Run -Id 53 -EventName 'pull_request')
        (New-Run -Id 54 -Path '.github/workflows/build.yml')
        (New-Run -Id 55 -TagName 'V0.1.0-preview.22')
        $run
    ) })
    (New-Response '/actions/runs/42' $run)
    (New-Response $jobsPath $assembly)
)

Test-Wait 'duplicates choose newest timestamp, then numeric ID' -Responses @(
    (New-Response $discoveryPath @{ workflow_runs = @(
        (New-Run -Id 100 -Created '2026-09-22T10:00:00Z')
        $run
        (New-Run -Id 9)
        (New-Run -Id 41)
    ) })
    (New-Response '/actions/runs/42' $run)
    (New-Response $jobsPath $assembly)
) -ExpectedWarning '*choosing newest run 42*'

Test-Wait 'newest failed run is not replaced by older success' -Responses @(
    (New-Response $discoveryPath @{ workflow_runs = @(
        (New-Run -Id 41 -Created '2026-09-22T10:00:00Z' -Status completed -Conclusion success)
        $run
    ) })
    (New-Response '/actions/runs/42' $run)
    (New-Response $jobsPath @{ jobs = @(New-Job -Conclusion failure) })
) -ExpectedWarning '*choosing newest run 42*' -ExpectedError "*job 'assemble-release' concluded 'failure'*"

Test-Wait 'discovery waits for a matching run' -Responses @(
    (New-Response $discoveryPath @{ workflow_runs = @() })
    (New-Response $discoveryPath @{ workflow_runs = @($run) })
    (New-Response '/actions/runs/42' $run)
    (New-Response $jobsPath $assembly)
) -ExpectedSleeps @(30)

Test-Wait 'discovery timeout' -Responses @(
    (New-Response $discoveryPath @{ workflow_runs = @() })
    (New-Response $discoveryPath @{ workflow_runs = @() })
) -ExpectedSleeps @(30) -ExpectedError '*not found after 2 discovery attempts*'

Test-Wait 'discovery follows pagination' -Responses @(
    (New-Response $discoveryPath @{ workflow_runs = @(1..100 | ForEach-Object { New-Run -Id $_ -TagName 'another-tag' }) })
    (New-Response ($discoveryPath.Replace('&page=1', '&page=2')) @{ workflow_runs = @($run) })
    (New-Response '/actions/runs/42' $run)
    (New-Response $jobsPath $assembly)
)

foreach ($jobName in @('assemble-release', 'github-release')) {
    foreach ($conclusion in @('failure', 'cancelled', 'skipped', 'timed_out')) {
        Test-Wait "$jobName rejects $conclusion" -Arguments @{ RunId = 42; JobName = $jobName; ExportRunId = $true } -Responses @(
            (New-Response '/actions/runs/42' $run)
            (New-Response $jobsPath @{ jobs = @(New-Job -Name $jobName -Conclusion $conclusion) })
        ) -ExpectedError "*job '$jobName' concluded '$conclusion'*"
    }
}

Test-Wait 'fixed RunId skips discovery and stays pinned through upload' -Arguments @{ RunId = 42; JobName = 'github-release' } -Responses @(
    (New-Response '/actions/runs/42' $run)
    (New-Response $jobsPath @{ jobs = @(New-Job -Name 'github-release' -Status in_progress -Conclusion $null) })
    (New-Response '/actions/runs/42' (New-Run -Status completed -Conclusion success))
    (New-Response $jobsPath $upload)
    (New-Response "/releases/tags/$tag" $release)
) -ExpectedSleeps @(30)

Test-Wait 'upload completion does not require an assets property' -Arguments @{ RunId = 42; JobName = 'github-release' } -Responses @(
    (New-Response '/actions/runs/42' $run)
    (New-Response $jobsPath $upload)
    (New-Response "/releases/tags/$tag" @{ tag_name = $tag; target_commitish = $sha })
)

foreach ($badRun in @(
    (New-Run -Id 43)
    (New-Run -TagName 'another-tag')
    (New-Run -ShaValue ('b' * 40))
    (New-Run -EventName 'workflow_dispatch')
    (New-Run -Path '.github/workflows/build.yml')
)) {
    Test-Wait "pinned identity rejects $($badRun | ConvertTo-Json -Compress)" -Arguments @{ RunId = 42 } -Responses @(
        (New-Response '/actions/runs/42' $badRun)
    ) -ExpectedError '*does not match*'
}

Test-Wait 'run identity is revalidated on every poll' -Arguments @{ RunId = 42 } -Responses @(
    (New-Response '/actions/runs/42' $run)
    (New-Response $jobsPath @{ jobs = @() })
    (New-Response '/actions/runs/42' (New-Run -ShaValue ('b' * 40)))
) -ExpectedSleeps @(30) -ExpectedError '*does not match*'

foreach ($conclusion in @('failure', 'cancelled', 'success')) {
    Test-Wait "refreshed completed run without job fails ($conclusion)" -Arguments @{ RunId = 42 } -Responses @(
        (New-Response '/actions/runs/42' $run)
        (New-Response $jobsPath @{ jobs = @() })
        (New-Response '/actions/runs/42' (New-Run -Status completed -Conclusion $conclusion))
        (New-Response $jobsPath @{ jobs = @() })
    ) -ExpectedSleeps @(30) -ExpectedError "*concluded '$conclusion' without a successful 'assemble-release' job*"
}

Test-Wait 'assembly job success is authoritative before workflow finishes' -Arguments @{ RunId = 42; ExportRunId = $true } -DotSource -Responses @(
    (New-Response '/actions/runs/42' $run)
    (New-Response $jobsPath $assembly)
)

Test-Wait 'job polling follows pagination' -Arguments @{ RunId = 42 } -Responses @(
    (New-Response '/actions/runs/42' $run)
    (New-Response $jobsPath @{ jobs = @(1..100 | ForEach-Object { New-Job -Name "other-$_" }) })
    (New-Response ($jobsPath.Replace('&page=1', '&page=2')) $assembly)
)

Test-Wait 'ambiguous job result fails closed' -Arguments @{ RunId = 42 } -Responses @(
    (New-Response '/actions/runs/42' $run)
    (New-Response $jobsPath @{ jobs = @((New-Job), (New-Job)) })
) -ExpectedError "*multiple 'assemble-release' jobs*"

foreach ($jobName in @('assemble-release', 'github-release')) {
    Test-Wait "$jobName polling timeout" -Arguments @{ RunId = 42; JobName = $jobName } -Responses @(
        (New-Response '/actions/runs/42' $run)
        (New-Response $jobsPath @{ jobs = @() })
        (New-Response '/actions/runs/42' $run)
        (New-Response $jobsPath @{ jobs = @() })
    ) -ExpectedSleeps @(30) -ExpectedError "*job '$jobName' did not succeed after 2 polling attempts*"
}

foreach ($badRelease in @(
    @{ tag_name = 'wrong-tag'; target_commitish = $sha }
    @{ tag_name = $tag; target_commitish = ('b' * 40) }
)) {
    Test-Wait 'release identity still fails closed' -Arguments @{ RunId = 42; JobName = 'github-release' } -Responses @(
        (New-Response '/actions/runs/42' $run)
        (New-Response $jobsPath $upload)
        (New-Response "/releases/tags/$tag" $badRelease)
    ) -ExpectedError '*GitHub release*does not match the exact tag and source SHA*'
}

Test-Wait 'transient API failure is retried' -Arguments @{ RunId = 42 } -Responses @(
    (New-Response '/actions/runs/42' $null 'temporary outage')
    (New-Response '/actions/runs/42' $run)
    (New-Response $jobsPath $assembly)
) -ExpectedSleeps @(10) -ExpectedWarning '*GitHub API request failed; retrying*'

Test-Wait 'API retry exhaustion is explicit' -Arguments @{ RunId = 42 } -Responses @(
    (New-Response '/actions/runs/42' $null 'unavailable')
    (New-Response '/actions/runs/42' $null 'unavailable')
    (New-Response '/actions/runs/42' $null 'unavailable')
) -ExpectedSleeps @(10, 20) -ExpectedWarning '*GitHub API request failed; retrying*' -ExpectedError '*GitHub API request failed*unavailable*'

foreach ($invalidId in @('', '$(pythonRunId)', '0', '-1')) {
    Test-Wait "invalid pinned ID cannot fall back to discovery ($invalidId)" -Arguments @{ RunId = $invalidId } -Responses @() `
        -ExpectedError '*RunId*'
}

# Keep these checks scoped to the active stage blocks rather than unrelated YAML text.
$pipeline = Get-Content -LiteralPath (Join-Path $repositoryRoot '.pipelines\release.yml') -Raw
function Get-Stage {
    param([string]$Name, [int]$Indent = 4)
    $match = [regex]::Match($pipeline, "(?ms)^ {$Indent}- stage: $Name\r?`n.*?(?=^ {$Indent}- |\z)")
    Assert-True $match.Success "Missing stage $Name"
    return $match.Value
}
function Assert-Dependencies {
    param([string]$Stage, [string[]]$Expected, [int]$Indent = 4)
    $block = Get-Stage $Stage $Indent
    $match = [regex]::Match($block, "(?m)^ {$($Indent + 2)}dependsOn:\r?`n(?<items>(?: {$($Indent + 4)}- [^\r\n]+\r?`n)+)")
    $actual = @([regex]::Matches($match.Groups['items'].Value, '- (\w+)') | ForEach-Object { $_.Groups[1].Value })
    Assert-True (($actual -join ',') -ceq ($Expected -join ',')) "Incorrect $Stage dependencies"
}
$wait = Get-Stage 'Wait_Python'
$collect = Get-Stage 'Collect_Python'
Assert-True ($wait.Contains('- checkout: self') -and $wait.Contains('name: WaitPython') -and
    $wait.Contains('filePath: eng/release/wait_python_release.ps1') -and
    $wait.Contains('-Version "$(version)" -SourceSha "$(Build.SourceVersion)"') -and
    $wait.Contains('-JobName assemble-release -ExportRunId')) 'Wait_Python must export the selected run'
Assert-Dependencies 'Collect_Python' @('Build', 'Wait_Python', 'Release_GitHub')
Assert-True ($collect.Contains("pythonRunId: `$[ stageDependencies.Wait_Python.wait_python.outputs['WaitPython.pythonRunId'] ]") -and
    $collect.Contains('filePath: eng/release/wait_python_release.ps1') -and
    $collect.Contains('-Version "$(version)" -SourceSha "$(Build.SourceVersion)"') -and
    $collect.Contains('-JobName github-release -RunId "$(pythonRunId)"')) 'Collect_Python must consume the frozen run output'
Assert-True ($collect.Contains('task: DownloadGitHubRelease@0') -and
    $collect.Contains("connection: 'github-service-connection'") -and
    $collect.Contains("version: 'v`$(version)'") -and
    $collect.Contains('verify_python_release.py release-set') -and
    $collect.Contains('--version "$(version)"') -and
    $collect.Contains('artifactName: python-packages')) 'Authenticated download and strict release-set verification must remain'
Assert-True ($collect.IndexOf('-JobName github-release') -lt $collect.IndexOf('task: DownloadGitHubRelease@0') -and
    $collect.IndexOf('task: DownloadGitHubRelease@0') -lt $collect.IndexOf('verify_python_release.py release-set') -and
    $collect.IndexOf('verify_python_release.py release-set') -lt $collect.IndexOf('artifactName: python-packages')) 'Collect order must remain wait, download, verify, freeze'
Assert-Dependencies 'Release_Npm' @('Build', 'Collect_Python') 6
Assert-Dependencies 'Release_PyPI' @('Build', 'Collect_Python', 'Release_Npm') 6
Assert-True ($pipeline.Contains("- `${{ if eq(parameters.DoEsrp, 'true') }}:") -and
    $pipeline.Contains("- `${{ if and(eq(parameters.DoEsrp, 'true'), eq(parameters.PublishPyPI, 'true')) }}:") -and
    -not $pipeline.Contains('ManualValidation@')) 'Automatic publication conditions must remain unchanged'
Write-Host "PASS: pipeline output wiring, collection order and automatic publication dependencies"
Write-Host "$caseCount Python workflow wait regression cases and pipeline checks passed (PowerShell $($PSVersionTable.PSVersion))"
