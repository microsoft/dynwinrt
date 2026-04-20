#!/usr/bin/env pwsh
# Copyright (c) Microsoft Corporation.
# Licensed under the MIT License.
#
# E2E test orchestrator: build, generate, run language-specific runners, collect results.
# All test logic lives in runners/py_runner.py and runners/ts_runner.ts.
#
# Usage:
#   .\tests\e2e_test.ps1                    # Full (build + generate + test)
#   .\tests\e2e_test.ps1 -SkipBuild         # Skip build step
#   .\tests\e2e_test.ps1 -Lang py           # Python only
#   .\tests\e2e_test.ps1 -Lang ts           # TypeScript only

param(
    [switch]$SkipBuild,
    [string[]]$Lang = @("py", "ts")
)

$ErrorActionPreference = "Stop"
$root = Split-Path $PSScriptRoot -Parent
$specsFile = Join-Path $PSScriptRoot "e2e_specs.json"
$e2eDir = Join-Path $root "tests\e2e_generated"
$runnersDir = Join-Path $root "tests\runners"

$env:PATH = "$env:USERPROFILE\.cargo\bin;$env:PATH"

Write-Host "=== dynwinrt E2E Test ===" -ForegroundColor Cyan

# --------------------------------------------------------------------------
# Detect available tools
# --------------------------------------------------------------------------
$hasPython = [bool](Get-Command python -ErrorAction SilentlyContinue)
$hasNode = [bool](Get-Command node -ErrorAction SilentlyContinue)

if ("py" -in $Lang -and -not $hasPython) {
    Write-Host "  SKIP Python (not installed)" -ForegroundColor DarkYellow
    $Lang = $Lang | Where-Object { $_ -ne "py" }
}
if ("ts" -in $Lang -and -not $hasNode) {
    Write-Host "  SKIP TypeScript (Node.js not installed)" -ForegroundColor DarkYellow
    $Lang = $Lang | Where-Object { $_ -ne "ts" }
}
if ($Lang.Count -eq 0) { Write-Error "No languages available"; exit 1 }

# --------------------------------------------------------------------------
# Build (optional)
# --------------------------------------------------------------------------
if (-not $SkipBuild) {
    Write-Host "`n--- Build ---" -ForegroundColor Yellow

    cargo build -p dynwinrt-codegen --release
    if ($LASTEXITCODE -ne 0) { Write-Error "dynwinrt-codegen build failed"; exit 1 }

    if ("py" -in $Lang) {
        Push-Location (Join-Path $root "bindings\py")
        if (-not (Test-Path .venv)) {
            python -m venv .venv
            .\.venv\Scripts\Activate.ps1
            pip install pytest maturin --quiet
        } else {
            .\.venv\Scripts\Activate.ps1
        }
        maturin build --quiet 2>&1 | Out-Null
        if ($LASTEXITCODE -ne 0) { Write-Error "maturin build failed"; exit 1 }
        $wheelDir = if ($env:CARGO_TARGET_DIR) { Join-Path $env:CARGO_TARGET_DIR "wheels" } else { Join-Path $root "target\wheels" }
        $whl = (Get-ChildItem (Join-Path $wheelDir "*.whl") | Sort-Object LastWriteTime -Descending | Select-Object -First 1).FullName
        if (-not $whl) { Write-Error "No wheel found after maturin build"; exit 1 }
        pip install $whl --force-reinstall --quiet 2>&1 | Out-Null
        if ($LASTEXITCODE -ne 0) { Write-Error "pip install failed"; exit 1 }
        Pop-Location
    }

    if ("ts" -in $Lang) {
        Push-Location (Join-Path $root "bindings\js")
        npm install --quiet 2>&1 | Out-Null
        if ($LASTEXITCODE -ne 0) { Write-Error "npm install failed"; exit 1 }
        npx napi build --no-const-enum --platform --release -o dist 2>&1 | Out-Null
        if ($LASTEXITCODE -ne 0) { Write-Error "napi build failed"; exit 1 }
        Pop-Location
    }
} else {
    $venvActivate = Join-Path $root "bindings\py\.venv\Scripts\Activate.ps1"
    if (Test-Path $venvActivate) { & $venvActivate }
}

# --------------------------------------------------------------------------
# Read specs & determine what to generate
# --------------------------------------------------------------------------
$specs = (Get-Content $specsFile -Raw | ConvertFrom-Json).specs
$skipped = $specs | Where-Object { $_.skip_reason }
$active = $specs | Where-Object { -not $_.skip_reason }

if ($skipped) {
    foreach ($s in $skipped) {
        Write-Host "  SKIP $($s.namespace).$($s.class): $($s.skip_reason)" -ForegroundColor DarkYellow
    }
}

# --------------------------------------------------------------------------
# Generate code
# --------------------------------------------------------------------------
if (Test-Path $e2eDir) { Remove-Item -Recurse -Force $e2eDir }

$winrtMeta = "cargo run -p dynwinrt-codegen --release --quiet --"

function Generate($lang, $outDir) {
    $langSpecs = $active | Where-Object { ($(if ($_.langs) { $_.langs } else { @("py","ts") })) -contains $lang }
    # codegen now uses "js" instead of "ts" — map accordingly
    $codegenLang = if ($lang -eq "ts") { "js" } else { $lang }
    $byNs = @{}
    foreach ($s in $langSpecs) {
        if (-not $byNs[$s.namespace]) { $byNs[$s.namespace] = @() }
        $classes = @($s.class)
        if ($s.extra_classes) { $classes += $s.extra_classes }
        $byNs[$s.namespace] += $classes
    }
    foreach ($ns in $byNs.Keys) {
        $classes = ($byNs[$ns] | Select-Object -Unique) -join ","
        Write-Host "  $lang`: $ns [$classes]"
        Invoke-Expression "$winrtMeta generate --namespace `"$ns`" --class-name `"$classes`" --lang $codegenLang --output `"$outDir`""
        if ($LASTEXITCODE -ne 0) { Write-Error "Generation failed: $ns ($lang)"; exit 1 }
    }
}

foreach ($l in $Lang) {
    Write-Host "`n--- Generate ($l) ---" -ForegroundColor Yellow
    Generate $l (Join-Path $e2eDir $l)
}

# --------------------------------------------------------------------------
# Run language-specific runners
# --------------------------------------------------------------------------
$totalPass = 0
$totalFail = 0
$allResults = @()

if ("py" -in $Lang) {
    Write-Host "`n--- Python E2E ---" -ForegroundColor Yellow
    $pyResult = Join-Path $e2eDir "results_py.json"
    python (Join-Path $runnersDir "py_runner.py") `
        --specs $specsFile `
        --generated (Join-Path $e2eDir "py") `
        --output $pyResult
    if ($LASTEXITCODE -ne 0) { $totalFail++ } else { $totalPass++ }
    if (Test-Path $pyResult) { $allResults += (Get-Content $pyResult -Raw | ConvertFrom-Json) }
}

if ("ts" -in $Lang) {
    Write-Host "`n--- TypeScript E2E ---" -ForegroundColor Yellow
    $tsResult = Join-Path $e2eDir "results_ts.json"
    $tsx = Join-Path $root "bindings\js\node_modules\.bin\tsx.cmd"
    if (-not (Test-Path $tsx)) {
        Write-Error "TypeScript E2E requires bindings/js/node_modules/.bin/tsx.cmd. Run npm install in bindings/js first."
        exit 1
    }
    & $tsx (Join-Path $runnersDir "ts_runner.ts") `
        --specs $specsFile `
        --generated (Join-Path $e2eDir "ts") `
        --runtime (Join-Path $root "bindings\js\dist\index.js") `
        --output $tsResult
    if ($LASTEXITCODE -ne 0) { $totalFail++ } else { $totalPass++ }
    if (Test-Path $tsResult) { $allResults += (Get-Content $tsResult -Raw | ConvertFrom-Json) }
}

# --------------------------------------------------------------------------
# Summary
# --------------------------------------------------------------------------
Write-Host "`n=== Summary ===" -ForegroundColor Cyan
foreach ($r in $allResults) {
    Write-Host "  $($r.language): $($r.passed)/$($r.total) passed"
}
if ($totalFail -eq 0) {
    Write-Host "ALL PASSED" -ForegroundColor Green
    Remove-Item -Recurse -Force $e2eDir
    exit 0
} else {
    Write-Host "SOME FAILED" -ForegroundColor Red
    exit 1
}


