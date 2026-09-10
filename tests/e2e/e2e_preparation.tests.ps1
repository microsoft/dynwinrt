#!/usr/bin/env pwsh
# Copyright (c) Microsoft Corporation.
# Licensed under the MIT License.

$ErrorActionPreference = "Stop"
$root = (Resolve-Path (Join-Path $PSScriptRoot "..\..")).Path
$scratch = Join-Path $root "target\e2e-preparation-$PID-$([Guid]::NewGuid().ToString('N'))"
$scripts = Join-Path $scratch "tests\e2e"
$trace = Join-Path $scratch "trace.json"
$venv = Join-Path $scratch "bindings\py\.venv\Scripts\python.exe"
$explicit = Join-Path $scratch "explicit-python.exe"
$standard = Join-Path $scripts "e2e_generated\retained.json"
$shell = (Get-Process -Id $PID).Path

try {
    New-Item -ItemType Directory -Path $scripts, (Split-Path $venv), (Split-Path $standard) -Force | Out-Null
    Copy-Item -LiteralPath (Join-Path $PSScriptRoot "e2e_test.ps1") -Destination $scripts
    New-Item -ItemType File -Path $venv, $explicit | Out-Null
    Set-Content -LiteralPath $standard -Value '{"retained": true}' -NoNewline

    # Execute the real entrypoint, intercepting only expensive external work.
    # Both boundaries deliberately fail: tests cannot mistake a stub runner or
    # stub build for successful generation/native E2E, and check exit propagation.
    @'
param([string[]]$Lang, [string]$Python, [string]$CargoProfile, [string]$CargoTarget, [switch]$KeepGenerated)
@{
    stage = "runner"; languages = @($Lang); python = $Python
    profile = $CargoProfile; target = $CargoTarget; keep = $KeepGenerated.IsPresent
} | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $PSScriptRoot "..\..\trace.json")
exit 23
'@ | Set-Content -LiteralPath (Join-Path $scripts "implementation_test.ps1")
    @'
param([string]$CaseFile)
$ErrorActionPreference = "Stop"
function cargo {
    @{ stage = "build"; arguments = @($args) } |
        ConvertTo-Json | Set-Content -LiteralPath (Join-Path $PSScriptRoot "trace.json")
    throw "Expected preparation build boundary"
}
function node { throw "Node must not run before the focused runner" }
$parameters = Get-Content -LiteralPath $CaseFile -Raw | ConvertFrom-Json -AsHashtable
if ($parameters.ContainsKey("HideGlobalTools")) {
    $parameters.Remove("HideGlobalTools")
    Remove-Item Function:\node
    $env:PATH = ""
}
& (Join-Path $PSScriptRoot "tests\e2e\e2e_test.ps1") @parameters
exit $LASTEXITCODE
'@ | Set-Content -LiteralPath (Join-Path $scratch "harness.ps1")

    function Invoke-PreparationCase {
        param([hashtable]$Parameters, [int]$ExpectedExit, [string]$Stage, [string]$ExpectedError = "requires -Lang py and/or ts")
        if (Test-Path -LiteralPath $trace) { Remove-Item -LiteralPath $trace }
        $caseFile = Join-Path $scratch "case.json"
        $Parameters | ConvertTo-Json | Set-Content -LiteralPath $caseFile
        $output = & $shell -NoProfile -File (Join-Path $scratch "harness.ps1") $caseFile 2>&1 | Out-String
        if ($LASTEXITCODE -ne $ExpectedExit) { throw "Unexpected exit $LASTEXITCODE (expected $ExpectedExit): $output" }
        if ((Get-Content -LiteralPath $standard -Raw) -ne '{"retained": true}') {
            throw "Focused suite changed the retained standard results"
        }
        if (-not $Stage) {
            if (Test-Path -LiteralPath $trace) { throw "Invalid selection entered preparation/dispatch" }
            if ($output -notlike "*$ExpectedError*") { throw "Unexpected selection error: $output" }
            return
        }
        if (-not (Test-Path -LiteralPath $trace)) { throw "Did not reach $Stage boundary: $output" }
        $result = Get-Content -LiteralPath $trace -Raw | ConvertFrom-Json
        if ($result.stage -ne $Stage) { throw "Expected $Stage before $($result.stage): $output" }
        return $result
    }

    $built = Invoke-PreparationCase @{
        Suite = "implementations"; Lang = @("py", "ts")
        CargoProfile = "coverage"; CargoTarget = "x86_64-pc-windows-msvc"
    } 1 "build"
    if (($built.arguments -join " ") -ne "build -p dynwinrt-codegen --profile coverage --target x86_64-pc-windows-msvc") {
        throw "Focused build lost profile/target arguments: $($built.arguments)"
    }
    $release = Invoke-PreparationCase @{ Suite = "implementations"; Lang = @("py") } 1 "build"
    if (($release.arguments -join " ") -ne "build -p dynwinrt-codegen --release") {
        throw "Focused build lost its release default: $($release.arguments)"
    }
    $selected = Invoke-PreparationCase @{
        Suite = "implementations"; Lang = @("py", "ts"); SkipBuild = $true
        CargoProfile = "dev"; CargoTarget = "aarch64-pc-windows-msvc"; KeepGenerated = $true
    } 23 "runner"
    if ($selected.python -ne $venv -or ($selected.languages -join ",") -ne "py,ts" -or
        $selected.profile -ne "dev" -or $selected.target -ne "aarch64-pc-windows-msvc" -or -not $selected.keep) {
        throw "Focused dispatch lost shared venv/language/profile/target/retention selection"
    }
    $selected = Invoke-PreparationCase @{
        Suite = "implementations"; Lang = @("py"); SkipBuild = $true; Python = $explicit
    } 23 "runner"
    if ($selected.python -ne $explicit -or $selected.keep) { throw "Explicit -Python did not override the existing venv" }
    $selected = Invoke-PreparationCase @{
        Suite = "implementations"; Lang = @("ts"); SkipBuild = $true
    } 23 "runner"
    if (($selected.languages -join ",") -ne "ts") { throw "Node-only focused selection was changed" }
    $selected = Invoke-PreparationCase @{
        Suite = "implementations"; Lang = @("py"); SkipBuild = $true; HideGlobalTools = $true
    } 23 "runner"
    if ($selected.python -ne $venv) { throw "Existing venv requires an unrelated global Python" }
    Invoke-PreparationCase @{ Suite = "implementations"; Lang = @("com"); SkipBuild = $true } 1 ""
    Remove-Item -LiteralPath $venv
    Invoke-PreparationCase @{
        Suite = "implementations"; Lang = @("py", "ts"); SkipBuild = $true; HideGlobalTools = $true
    } 1 "" "No languages available"
    Write-Host "Focused E2E preparation regressions passed."
} finally {
    if (Test-Path -LiteralPath $scratch) { Remove-Item -LiteralPath $scratch -Recurse -Force }
}
exit 0
