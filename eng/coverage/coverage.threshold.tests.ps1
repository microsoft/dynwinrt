#!/usr/bin/env pwsh
# Copyright (c) Microsoft Corporation.
# Licensed under the MIT License.

$ErrorActionPreference = "Stop"
$root = (Resolve-Path (Join-Path $PSScriptRoot "..\..")).Path
$coverageScript = Join-Path $root "eng\coverage\coverage.ps1"
$fixture = Join-Path $root "eng\coverage\testdata\threshold-baseline"

function Assert-ExpectedFailure {
    param(
        [scriptblock]$Command,
        [string]$ExpectedMessage
    )

    try {
        & $Command
    } catch {
        if ($_.Exception.Message -like "*$ExpectedMessage*") {
            return
        }
        throw
    }

    throw "Expected failure containing: $ExpectedMessage"
}

Write-Host "Validating default coverage thresholds against the recorded baseline..."
& $coverageScript `
    -OutputDirectory $fixture `
    -ValidateOnly

Write-Host "Validating Rust threshold failures..."
Assert-ExpectedFailure {
    & $coverageScript `
        -OutputDirectory $fixture `
        -ValidateOnly `
        -MinRustLineCoverage 47 `
        -MinPythonLineCoverage 70 `
        -MinJavaScriptLineCoverage 18
} "Rust line coverage 46.67% is below the required 47%"

Write-Host "Validating JavaScript threshold failures..."
Assert-ExpectedFailure {
    & $coverageScript `
        -OutputDirectory $fixture `
        -ValidateOnly `
        -MinRustLineCoverage 45 `
        -MinPythonLineCoverage 70 `
        -MinJavaScriptLineCoverage 19
} "JavaScript aggregate line coverage 18.77% is below the required 19%"

Write-Host "Validating Python threshold failures..."
Assert-ExpectedFailure {
    & $coverageScript `
        -OutputDirectory $fixture `
        -ValidateOnly `
        -MinPythonLineCoverage 73
} "Generated Python line coverage 72.27% is below the required 73%"

# Load report helpers without running builds or touching native binding outputs.
$ast = [Management.Automation.Language.Parser]::ParseFile($coverageScript, [ref]$null, [ref]$null)
foreach ($name in @(
    "Get-JavaScriptCoverageLayers",
    "Test-LcovSourceCovered",
    "Assert-NativeSourceCoverage",
    "Assert-MinimumCoverage",
    "Get-CoverageSummaryMarkdown",
    "Write-CoverageSummary"
)) {
    $function = $ast.Find({
        param($node)
        $node -is [Management.Automation.Language.FunctionDefinitionAst] -and $node.Name -eq $name
    }, $false)
    if (-not $function) { throw "Missing coverage report helper: $name" }
    . ([scriptblock]::Create($function.Extent.Text))
}

Write-Host "Validating JavaScript source families..."
$jsReport = Join-Path $fixture "javascript"
$jsRuntimeReport = Join-Path $jsReport "runtime"
$jsWinrtReport = Join-Path $jsReport "generated-winrt"
$jsImplementationReport = Join-Path $jsReport "generated-implementations"
$jsComReport = Join-Path $jsReport "generated-classic-com"
$winrtCoverageExpected = $true
$comCoverageExpected = $true
$layers = @(Get-JavaScriptCoverageLayers)
$includes = @($layers | ForEach-Object { $_.Includes })
foreach ($expected in @(
    "bindings/js/dist/**/*.js",
    "tests/e2e/e2e_generated/ts/**/*.js",
    "tests/e2e/e2e_generated/implementations/js/**/*.js",
    "tests/e2e/e2e_generated/com/**/*.js"
)) {
    if ($includes -notcontains $expected) { throw "Missing JavaScript source family: $expected" }
}
if ($layers.Count -ne 4) { throw "Expected four distinct JavaScript source families" }
$comCoverageExpected = $false
if (@(Get-JavaScriptCoverageLayers).Count -ne 3) { throw "Skipping COM must preserve implementation coverage" }
$winrtCoverageExpected = $false
if (@(Get-JavaScriptCoverageLayers).Count -ne 1) { throw "Skipping E2E must preserve runtime coverage" }
if (@(Get-JavaScriptCoverageLayers -All).Count -ne 4) { throw "Summary must discover every generated family" }

Write-Host "Validating native source execution checks..."
$pattern = "bindings[\\/]py[\\/]src[\\/]implementation\.rs"
foreach ($separator in @("/", "\")) {
    $source = (@("bindings", "py", "src", "implementation.rs") -join $separator)
    if (-not (Test-LcovSourceCovered "SF:$source`r`nLH:1`r`nend_of_record`r`n" $pattern)) {
        throw "Covered native implementation source was not detected: $source"
    }
    if (Test-LcovSourceCovered "SF:$source`nLH:0`nend_of_record`n" $pattern) {
        throw "An unexecuted native source must not satisfy coverage presence checks"
    }
}

function New-NativeLcovFixture {
    param(
        [string[]]$Sources,
        [string[]]$UncoveredSources = @(),
        [string]$Prefix = "",
        [string]$Separator = "/",
        [string]$NewLine = "`n"
    )
    return (@(
        foreach ($source in $Sources) {
            $path = if ($source.EndsWith("/")) { "${source}mod.rs" } else { $source }
            $path = ($Prefix + $path).Replace("/", $Separator)
            $hits = if ($UncoveredSources -contains $source) { 0 } else { 1 }
            @("SF:$path", "DA:1,$hits", "LF:1", "LH:$hits", "end_of_record", "") -join $NewLine
        }
    ) -join "")
}

Write-Host "Validating split native binding source guards..."
$nativeSources = @(
    "bindings/js/src/winrt_types.rs",
    "bindings/js/src/winrt_methods.rs",
    "bindings/js/src/value.rs",
    "bindings/js/src/winrt_array.rs",
    "bindings/js/src/winrt_struct.rs",
    "bindings/js/src/js_storage.rs",
    "bindings/js/src/com_value.rs",
    "bindings/py/src/runtime.rs"
)
$implementationSources = @(
    "bindings/js/src/winrt_implementation.rs",
    "bindings/py/src/implementation.rs",
    "crates/dynwinrt/src/winrt_implementation/"
)
$wiringSource = "bindings/js/src/lib.rs"
$unrelatedSources = @(
    $wiringSource,
    "bindings/js/src/benchmarks.rs",
    "bindings/py/src/value.rs",
    "tests/e2e/e2e_generated/ts/value.js"
)
foreach ($separator in @("/", "\")) {
    foreach ($prefix in @("", "D:/a/dynwinrt/dynwinrt/")) {
        foreach ($newLine in @("`n", "`r`n")) {
            $format = @{ Separator = $separator; Prefix = $prefix; NewLine = $newLine }
            foreach ($requireImplementation in @($false, $true)) {
                $requiredSources = @($nativeSources)
                if ($requireImplementation) { $requiredSources += $implementationSources }

                $lcov = New-NativeLcovFixture -Sources $requiredSources @format
                Assert-NativeSourceCoverage -Lcov $lcov -RequireWinrtImplementation:$requireImplementation
                $lcov = New-NativeLcovFixture `
                    -Sources ($requiredSources + $wiringSource) -UncoveredSources $wiringSource @format
                Assert-NativeSourceCoverage -Lcov $lcov -RequireWinrtImplementation:$requireImplementation

                foreach ($requiredSource in $requiredSources) {
                    $remainingSources = @($requiredSources | Where-Object { $_ -ne $requiredSource })
                    $lcov = New-NativeLcovFixture -Sources ($remainingSources + $unrelatedSources) @format
                    Assert-ExpectedFailure {
                        Assert-NativeSourceCoverage -Lcov $lcov -RequireWinrtImplementation:$requireImplementation
                    } "Rust coverage did not execute native binding source: $requiredSource"

                    $lcov = New-NativeLcovFixture `
                        -Sources ($requiredSources + $unrelatedSources) -UncoveredSources $requiredSource @format
                    Assert-ExpectedFailure {
                        Assert-NativeSourceCoverage -Lcov $lcov -RequireWinrtImplementation:$requireImplementation
                    } "Rust coverage did not execute native binding source: $requiredSource"
                }
            }
        }
    }
}

Write-Host "Validating failed gates retain the complete summary..."
$output = Join-Path $PSScriptRoot "testdata\.summary-test-$([guid]::NewGuid().ToString('N'))"
$rustReport = Join-Path $fixture "rust"
$pythonReport = Join-Path $fixture "python"
$pythonRuntimeReport = Join-Path $pythonReport "runtime"
$pythonWinrtReport = Join-Path $pythonReport "generated-winrt"
$pythonImplementationReport = Join-Path $pythonReport "generated-implementations"
$MinRustLineCoverage = 45
$MinPythonLineCoverage = 73
$MinJavaScriptLineCoverage = 18
New-Item -ItemType Directory -Path $output | Out-Null
try {
    Assert-ExpectedFailure { Write-CoverageSummary } "Generated Python line coverage 72.27%"
    $summary = Get-Content -LiteralPath (Join-Path $output "summary.md") -Raw
    if ($summary -notmatch "Python aggregate.*72\.27%" -or $summary -notmatch "JavaScript aggregate.*18\.77%") {
        throw "A failing Python gate hid coverage diagnostics for another layer"
    }

    Write-Host "Validating the unchanged default Python 70% boundary..."
    $boundaryDirectory = Join-Path $output "python"
    New-Item -ItemType Directory -Path $boundaryDirectory | Out-Null
    $boundary = @{
        totals = @{
            covered_lines = 70
            num_statements = 100
            covered_branches = 0
            num_branches = 0
        }
    }
    $boundary | ConvertTo-Json -Depth 3 |
        Set-Content -LiteralPath (Join-Path $boundaryDirectory "coverage.json")
    & $coverageScript -OutputDirectory $output -ValidateOnly
    $boundary.totals.covered_lines = 69
    $boundary | ConvertTo-Json -Depth 3 |
        Set-Content -LiteralPath (Join-Path $boundaryDirectory "coverage.json")
    Assert-ExpectedFailure {
        & $coverageScript -OutputDirectory $output -ValidateOnly
    } "Generated Python line coverage 69% is below the required 70%"
} finally {
    Remove-Item -LiteralPath $output -Recurse -Force
}

Write-Host "Coverage threshold validation passed."
