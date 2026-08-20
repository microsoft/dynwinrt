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
        -MinJavaScriptLineCoverage 20
} "Rust line coverage 46.67% is below the required 47%"

Write-Host "Validating JavaScript threshold failures..."
Assert-ExpectedFailure {
    & $coverageScript `
        -OutputDirectory $fixture `
        -ValidateOnly `
        -MinRustLineCoverage 45 `
        -MinPythonLineCoverage 70 `
        -MinJavaScriptLineCoverage 23
} "JavaScript aggregate line coverage 22.19% is below the required 23%"

Write-Host "Coverage threshold validation passed."
