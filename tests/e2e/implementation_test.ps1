#!/usr/bin/env pwsh
# Copyright (c) Microsoft Corporation.
# Licensed under the MIT License.
#
# Generation, strict consumer typechecks, and isolated native-vtable scenarios.
# Use e2e_test.ps1 -Suite implementations for the normal build/skip-build entrypoint.
# This script intentionally never installs tools, activates UI, or registers a task.

param(
    [ValidateSet("py", "ts")]
    [string[]]$Lang = @("py", "ts"),
    [string]$Python = "python",
    [string]$CargoProfile = "release",
    [string]$CargoTarget,
    [switch]$KeepGenerated
)

$ErrorActionPreference = "Stop"
$root = Split-Path (Split-Path $PSScriptRoot -Parent) -Parent
$out = Join-Path $PSScriptRoot "e2e_generated\implementations"
$runtime = Join-Path $root "bindings\js\dist\winrt.js"
$failed = 0
$generationResults = @{}
$typecheckResults = @{}
[string[]]$cargoArgs = @(
    if ($CargoProfile -eq "release") { "--release" }
    else { "--profile"; $CargoProfile }
    if ($CargoTarget) { "--target"; $CargoTarget }
)
$winmd = if ($env:DYNWINRT_WINDOWS_WINMD) {
    $env:DYNWINRT_WINDOWS_WINMD
} else {
    "C:\Program Files (x86)\Windows Kits\10\UnionMetadata\10.0.26100.0\Windows.winmd"
}
if (-not (Test-Path -LiteralPath $winmd -PathType Leaf)) {
    throw "WinRT implementation E2E requires Windows.winmd. Set DYNWINRT_WINDOWS_WINMD."
}

# All are ordinary non-generic interfaces. IBindable* only contributes metadata;
# the test never initializes XAML. IDataReader contributes a real FillArray slot.
$classes = @(
    "Windows.ApplicationModel.Background.IBackgroundTask",
    "Windows.ApplicationModel.Background.IBackgroundTaskInstance",
    "Windows.Foundation.IStringable",
    "Windows.Foundation.IClosable",
    "Windows.Foundation.IMemoryBufferReference",
    "Windows.Globalization.NumberFormatting.INumberParser",
    "Windows.Foundation.IPropertyValue",
    "Windows.Storage.Streams.IDataWriter",
    "Windows.Storage.Streams.IBuffer",
    "Windows.Storage.Streams.IDataReader",
    "Windows.UI.Xaml.Interop.IBindableVectorView",
    "Windows.UI.Xaml.Interop.IBindableIterable",
    "Windows.UI.Xaml.Interop.IBindableIterator"
) -join ","

if (Test-Path -LiteralPath $out) { Remove-Item -LiteralPath $out -Recurse -Force }
New-Item -ItemType Directory -Path $out -Force | Out-Null

foreach ($language in $Lang) {
    Write-Host "`n--- Generate WinRT implementations ($language) ---" -ForegroundColor Yellow
    $directory = Join-Path $out $(if ($language -eq "py") { "python_bindings" } else { "js" })
    $codegenLanguage = if ($language -eq "py") { "py" } else { "js" }
    $runtimeImport = [IO.Path]::GetRelativePath($directory, $runtime).Replace("\", "/")
    $generatedModules = 0
    & cargo run -p dynwinrt-codegen @cargoArgs --quiet -- generate `
        --winmd $winmd --class-name $classes --lang $codegenLanguage `
        --output $directory --import-name $runtimeImport |
        ForEach-Object {
            if ($_ -like "Generated *") { $generatedModules++ }
            else { Write-Host $_ }
        }
    $generationResults[$language] = $LASTEXITCODE -eq 0
    if ($LASTEXITCODE -ne 0) {
        Write-Host "  FAIL generation ($language); other selected languages will still run." -ForegroundColor Red
        $failed++
        continue
    }
    Write-Host "  Generated $generatedModules modules in $directory"

    # Typecheck failures are not an excuse to skip native execution. Retain both
    # diagnostics so an integration run reports the complete blocked matrix.
    if ($language -eq "py") {
        Write-Host "`n--- Implementation .pyi consumer typecheck ---" -ForegroundColor Yellow
        $previousMypyPath = $env:MYPYPATH
        try {
            # Long generic delegate facade names can exceed MAX_PATH in a
            # shared-worktree checkout. Use extended paths, not registry/OS
            # changes or suppressed import diagnostics.
            $env:MYPYPATH = if ($env:OS -eq "Windows_NT" -and -not $out.StartsWith('\\?\')) {
                if ($out.StartsWith('\\')) { '\\?\UNC\' + $out.Substring(2) }
                else { '\\?\' + $out }
            } else { $out }
            & $Python -m mypy --strict (Join-Path $PSScriptRoot "typecheck\python_implementation_api.py")
            $typecheckResults[$language] = $LASTEXITCODE -eq 0
            if ($LASTEXITCODE -ne 0) { $failed++ }
        } finally {
            $env:MYPYPATH = $previousMypyPath
        }
        Write-Host "`n--- Python implementation native E2E ---" -ForegroundColor Yellow
        & $Python (Join-Path $PSScriptRoot "runners\implementation_py.py") `
            --generated $directory --output (Join-Path $out "results_py.json")
        if ($LASTEXITCODE -ne 0) { $failed++ }
    } else {
        Write-Host "`n--- Implementation .d.ts consumer typecheck ---" -ForegroundColor Yellow
        $tsc = Join-Path $root "bindings\js\node_modules\typescript\bin\tsc"
        if (-not (Test-Path -LiteralPath $tsc)) {
            Write-Host "  FAIL: TypeScript compiler missing at $tsc (install the existing bindings/js dependencies)." -ForegroundColor Red
            $failed++
            $typecheckResults[$language] = $false
        } else {
            & node $tsc --noEmit --strict --target ES2022 --module Node16 --moduleResolution Node16 `
                --types node --typeRoots (Join-Path $root "bindings\js\node_modules\@types") `
                (Join-Path $PSScriptRoot "typecheck\ts_implementation_api.ts")
            $typecheckResults[$language] = $LASTEXITCODE -eq 0
            if ($LASTEXITCODE -ne 0) { $failed++ }
        }
        Write-Host "`n--- Node implementation native E2E ---" -ForegroundColor Yellow
        & node (Join-Path $PSScriptRoot "runners\implementation_js.mjs") `
            --generated $directory --runtime $runtime --output (Join-Path $out "results_ts.json")
        if ($LASTEXITCODE -ne 0) { $failed++ }
    }
}

Write-Host "`n=== Generated implementation summary ===" -ForegroundColor Cyan
foreach ($language in $Lang) {
    $generationStatus = if ($generationResults[$language]) { "PASS" } else { "FAIL" }
    $typecheckStatus = if (-not $typecheckResults.ContainsKey($language)) { "BLOCKED" }
        elseif ($typecheckResults[$language]) { "PASS" } else { "FAIL" }
    Write-Host "  $language`: generation $generationStatus; strict typecheck $typecheckStatus"
    $result = Join-Path $out "results_$language.json"
    if (Test-Path -LiteralPath $result) {
        $report = Get-Content -LiteralPath $result -Raw | ConvertFrom-Json
        Write-Host "  $($report.language): $($report.passed)/$($report.total) passed ($($report.architecture))"
    } else {
        Write-Host "  $language`: BLOCKED before native runner produced results" -ForegroundColor Red
    }
}
if ($failed -ne 0) {
    Write-Host "IMPLEMENTATION E2E FAILED; generated files and JSON retained at $out" -ForegroundColor Red
    exit 1
}
Write-Host "IMPLEMENTATION E2E PASSED" -ForegroundColor Green
if (-not $KeepGenerated) { Remove-Item -LiteralPath $out -Recurse -Force }
exit 0
