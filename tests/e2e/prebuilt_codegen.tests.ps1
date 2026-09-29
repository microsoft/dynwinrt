#!/usr/bin/env pwsh
# Copyright (c) Microsoft Corporation.
# Licensed under the MIT License.

$ErrorActionPreference = "Stop"
# Capture the intentional nonzero generation boundaries before checking them.
$PSNativeCommandUseErrorActionPreference = $false
$root = (Resolve-Path (Join-Path $PSScriptRoot "..\..")).Path
$scratch = Join-Path $root "target\prebuilt-codegen-$PID-$([Guid]::NewGuid().ToString('N'))"
$scripts = Join-Path $scratch "tests\e2e"
$shell = (Get-Process -Id $PID).Path
$trace = Join-Path $scratch "trace.jsonl"
$prebuilt = Join-Path $scratch "artifact with spaces\codegen.ps1"
$python = Join-Path $scratch "python.ps1"
try {
    New-Item -ItemType Directory -Path $scripts, (Split-Path $prebuilt) -Force | Out-Null
    foreach ($file in @(
        "e2e_test.ps1",
        "implementation_test.ps1",
        "codegen.ps1",
        "e2e_specs.json",
        "check_generated_python.py"
    )) {
        Copy-Item -LiteralPath (Join-Path $PSScriptRoot $file) -Destination $scripts
    }
    Set-Content -LiteralPath (Join-Path $scratch "metadata.winmd") -Value "external metadata boundary"
    Set-Content -LiteralPath $python -Value '$global:LASTEXITCODE = 0'
    @'
@{ kind = "prebuilt"; arguments = @($args) } | ConvertTo-Json -Compress |
    Add-Content -LiteralPath $env:CODEGEN_TRACE
$outputIndex = [Array]::IndexOf($args, "--output")
if ($outputIndex -ge 0) { New-Item -ItemType Directory -Path $args[$outputIndex + 1] -Force | Out-Null }
$global:LASTEXITCODE = if (@(Get-Content $env:CODEGEN_TRACE).Count -eq [int]$env:CODEGEN_STOP) { 23 } else { 0 }
'@ | Set-Content -LiteralPath $prebuilt
    @'
param([string]$CaseFile)
$ErrorActionPreference = "Stop"
function cargo {
    @{ kind = "cargo"; arguments = @($args) } | ConvertTo-Json -Compress | Add-Content $env:CODEGEN_TRACE
    $global:LASTEXITCODE = 31
}
$case = Get-Content -LiteralPath $CaseFile -Raw | ConvertFrom-Json -AsHashtable
$env:DYNWINRT_CODEGEN = $case.environmentCodegen
$env:DYNWINRT_WIN32_WINMD = Join-Path $PSScriptRoot "metadata.winmd"
$env:DYNWINRT_WINDOWS_WINMD = $env:DYNWINRT_WIN32_WINMD
$env:CODEGEN_TRACE = Join-Path $PSScriptRoot "trace.jsonl"
$env:CODEGEN_STOP = $case.stop
$parameters = $case.parameters
& (Join-Path $PSScriptRoot "tests\e2e\$($case.script)") @parameters
exit $LASTEXITCODE
'@ | Set-Content -LiteralPath (Join-Path $scratch "harness.ps1")

    function Invoke-GenerationCase([string]$Script, [hashtable]$Parameters, [int]$Stop, [string]$EnvironmentCodegen = "") {
        if (Test-Path $trace) { Remove-Item -LiteralPath $trace }
        $caseFile = Join-Path $scratch "case.json"
        @{
            script = $Script; parameters = $Parameters; stop = $Stop
            environmentCodegen = if ($EnvironmentCodegen) { $EnvironmentCodegen } else { $null }
        } |
            ConvertTo-Json -Depth 5 | Set-Content -LiteralPath $caseFile
        $output = & $shell -NoProfile -File (Join-Path $scratch "harness.ps1") $caseFile 2>&1 | Out-String
        if ($LASTEXITCODE -eq 0) { throw "The deliberately failing generation boundary became green: $output" }
        $records = @(Get-Content -LiteralPath $trace | ForEach-Object { $_ | ConvertFrom-Json })
        if ($records.Count -ne $Stop) { throw "Expected $Stop invocations, got $($records.Count): $output" }
        return $records
    }

    $active = (Get-Content (Join-Path $scripts "e2e_specs.json") -Raw | ConvertFrom-Json).specs |
        Where-Object { -not $_.skip_reason }
    $namespaces = @(
        foreach ($lang in @("py", "ts")) {
            $active | Where-Object { -not $_.langs -or $lang -in $_.langs } |
                Select-Object -ExpandProperty namespace -Unique
        }
    )
    $winrt = Invoke-GenerationCase "e2e_test.ps1" @{
        SkipBuild = $true; Suite = "standard"; Lang = @("py", "ts"); Python = $shell; Codegen = $prebuilt
    } $namespaces.Count
    $native = Invoke-GenerationCase "e2e_test.ps1" @{
        SkipBuild = $true; Suite = "standard"; Lang = @("com", "win32"); Python = $shell
    } 12 $prebuilt
    foreach ($record in @($winrt) + @($native)) {
        if ($record.kind -ne "prebuilt" -or $record.arguments[0] -ne "generate") {
            throw "Artifact mode invoked Cargo or lost generation arguments"
        }
    }
    if ("Windows.Media" -notin @($native | ForEach-Object {
        $index = [Array]::IndexOf($_.arguments, "--namespace")
        if ($index -ge 0) { $_.arguments[$index + 1] }
    })) { throw "COM interop lost WinRT generation" }
    if (($native[-1].arguments -join " ") -notlike "*Windows.Win32.System.Registry.Apis*") {
        throw "Flat Win32 generation did not use the prebuilt path"
    }
    foreach ($language in @("py", "ts")) {
        $implementation = Invoke-GenerationCase "implementation_test.ps1" @{
            Lang = @($language); Codegen = $prebuilt
        } 1
        if ($implementation.kind -ne "prebuilt") { throw "Implementation generation rebuilt codegen" }
    }
    $pythonNamespaces = @($active | Where-Object { -not $_.langs -or "py" -in $_.langs } |
        Select-Object -ExpandProperty namespace -Unique)
    $all = Invoke-GenerationCase "e2e_test.ps1" @{
        SkipBuild = $true; Suite = "all"; Lang = @("py"); Python = $python; Codegen = $prebuilt
    } ($pythonNamespaces.Count + 1)
    if ($all[-1].kind -ne "prebuilt" -or ($all[-1].arguments -join " ") -notlike "*IBackgroundTask*") {
        throw "The full suite did not forward prebuilt codegen into its implementation sub-suite"
    }
    $fallback = Invoke-GenerationCase "e2e_test.ps1" @{
        SkipBuild = $true; Suite = "standard"; Lang = @("py"); Python = $shell
        CargoProfile = "coverage"; CargoTarget = "x86_64-pc-windows-msvc"
    } 1
    if ($fallback.kind -ne "cargo" -or ($fallback.arguments -join " ") -notlike
        "run -p dynwinrt-codegen --profile coverage --target x86_64-pc-windows-msvc --quiet -- generate *") {
        throw "Omitted prebuilt override changed the developer/profile fallback"
    }
    Write-Host "Prebuilt WinRT/COM/Win32/implementation generation and Cargo fallback regressions passed."
} finally {
    if (Test-Path -LiteralPath $scratch) { Remove-Item -LiteralPath $scratch -Recurse -Force }
}
exit 0
