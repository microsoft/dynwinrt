#!/usr/bin/env pwsh
# Copyright (c) Microsoft Corporation.
# Licensed under the MIT License.

param(
    [string]$OutputDirectory = "artifacts\coverage",
    [string]$Python,
    [string]$CargoTarget,
    [string]$Win32Winmd = $env:DYNWINRT_WIN32_WINMD,
    [ValidateRange(0, 100)]
    [double]$MinRustLineCoverage = 45,
    [ValidateRange(0, 100)]
    [double]$MinPythonLineCoverage = 70,
    [ValidateRange(0, 100)]
    [double]$MinJavaScriptLineCoverage = 18,
    [switch]$ValidateOnly,
    [switch]$SkipE2E,
    [switch]$SkipCom
)

$ErrorActionPreference = "Stop"
$outputSyntax = $OutputDirectory.Replace("/", "\")
if (
    $outputSyntax.StartsWith("\\?\") -or
    $outputSyntax.StartsWith("\\.\") -or
    $outputSyntax.StartsWith("\??\")
) {
    throw "OutputDirectory cannot use Windows device-path syntax"
}
$root = (Resolve-Path (Join-Path $PSScriptRoot "..\..")).Path
$output = if ([IO.Path]::IsPathRooted($OutputDirectory)) {
    [IO.Path]::GetFullPath($OutputDirectory)
} else {
    [IO.Path]::GetFullPath((Join-Path $root $OutputDirectory))
}
$rustReport = Join-Path $output "rust"
$pythonReport = Join-Path $output "python"
$jsReport = Join-Path $output "javascript"
$jsRuntimeReport = Join-Path $jsReport "runtime"
$jsWinrtReport = Join-Path $jsReport "generated-winrt"
$jsComReport = Join-Path $jsReport "generated-classic-com"
$jsTemp = Join-Path $output "raw\javascript"
$rustRaw = Join-Path $output "raw\rust"
$pythonConfig = Join-Path $PSScriptRoot "python-coveragerc"
$repositoryArtifactRoot = Join-Path $root "artifacts"
$outputMarker = Join-Path $output ".dynwinrt-coverage-output"
$outputMarkerText = "Managed by eng/coverage/coverage.ps1"
$pipelineError = $null
$pythonStartup = $null
$originalLocation = (Get-Location).Path
$scratchRoot = Join-Path ([IO.Path]::GetTempPath()) "dynwinrt-coverage-$PID-$([guid]::NewGuid().ToString('N'))"
$pythonEnvironment = Join-Path $scratchRoot "python"
$jsDist = Join-Path $root "bindings\js\dist"
$jsDistBackup = Join-Path $scratchRoot "js-dist"
$jsDistPrepared = $false
$jsDistWasPresent = $false
$e2eGenerated = Join-Path $root "tests\e2e\e2e_generated"
$e2eGeneratedByCoverage = $false
$winrtCoverageExpected = $false
$comCoverageExpected = $false
$originalEnvironment = @{}
Get-ChildItem Env: | ForEach-Object {
    $originalEnvironment[$_.Name] = $_.Value
}

function Get-NormalizedPath {
    param([string]$Path)
    return [IO.Path]::GetFullPath($Path).TrimEnd(
        [IO.Path]::DirectorySeparatorChar,
        [IO.Path]::AltDirectorySeparatorChar
    )
}

function Test-PathContains {
    param(
        [string]$Container,
        [string]$Contained
    )
    $containerPath = Get-NormalizedPath $Container
    $containedPath = Get-NormalizedPath $Contained
    return (
        $containedPath.Equals($containerPath, [StringComparison]::OrdinalIgnoreCase) -or
        $containedPath.StartsWith(
            "$containerPath$([IO.Path]::DirectorySeparatorChar)",
            [StringComparison]::OrdinalIgnoreCase
        )
    )
}

function Assert-NoReparsePoint {
    param([string]$Path)
    $current = [IO.Path]::GetFullPath($Path)
    while ($current) {
        if (Test-Path -LiteralPath $current) {
            $item = Get-Item -LiteralPath $current -Force
            if (($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
                throw "OutputDirectory cannot traverse a reparse point: $current"
            }
        }
        $parent = [IO.Directory]::GetParent($current)
        $current = if ($parent) { $parent.FullName } else { $null }
    }
}

function Assert-NoReparsePointTree {
    param([string]$Path)
    if (-not (Test-Path -LiteralPath $Path)) {
        return
    }
    $items = @(
        Get-Item -LiteralPath $Path -Force
        Get-ChildItem -LiteralPath $Path -Force -Recurse
    )
    foreach ($item in $items) {
        if (($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
            throw "Refusing to clean a report tree containing a reparse point: $($item.FullName)"
        }
    }
}

Assert-NoReparsePoint $output
$outputCandidates = @($output)
if (Test-Path -LiteralPath $output) {
    $outputCandidates += (Resolve-Path -LiteralPath $output).Path
}
foreach ($candidate in ($outputCandidates | Select-Object -Unique)) {
    $filesystemRoot = [IO.Path]::GetPathRoot($candidate)
    if ((Get-NormalizedPath $candidate).Equals(
        (Get-NormalizedPath $filesystemRoot),
        [StringComparison]::OrdinalIgnoreCase
    )) {
        throw "OutputDirectory cannot be a filesystem root: $candidate"
    }
    if (-not $ValidateOnly) {
        foreach ($protectedPath in @($originalLocation, $root)) {
            if (Test-PathContains $candidate $protectedPath) {
                throw "OutputDirectory cannot contain protected directory: $protectedPath"
            }
        }
        if (
            (Test-PathContains $root $candidate) -and
            -not (Test-PathContains $repositoryArtifactRoot $candidate)
        ) {
            throw "OutputDirectory inside the repository must be under: $repositoryArtifactRoot"
        }
    }
}

function Invoke-Step {
    param(
        [string]$Name,
        [scriptblock]$Action
    )
    Write-Host "`n=== $Name ===" -ForegroundColor Cyan
    & $Action
    if ($LASTEXITCODE -ne 0) {
        throw "$Name failed with exit code $LASTEXITCODE"
    }
}

function Assert-MinimumCoverage {
    param(
        [string]$Name,
        [double]$Actual,
        [double]$Minimum
    )
    if ($Actual -lt $Minimum) {
        throw "$Name line coverage $Actual% is below the required $Minimum%"
    }
}

function New-LiteralDirectory {
    param([string[]]$Path)
    foreach ($directory in $Path) {
        [void][IO.Directory]::CreateDirectory($directory)
    }
}

function Resolve-Python {
    if ($Python) {
        return (Resolve-Path -LiteralPath $Python).Path
    }
    return (Get-Command python -ErrorAction Stop).Source
}

function Prepare-JavaScriptOutput {
    New-LiteralDirectory $scratchRoot
    if (Test-Path -LiteralPath $jsDist) {
        Move-Item -LiteralPath $jsDist -Destination $jsDistBackup
        $script:jsDistWasPresent = $true
    }
    $script:jsDistPrepared = $true
}

function Restore-JavaScriptOutput {
    if (-not $jsDistPrepared) {
        return
    }
    if (Test-Path -LiteralPath $jsDist) {
        Remove-Item -LiteralPath $jsDist -Recurse -Force
    }
    if ($jsDistWasPresent) {
        Move-Item -LiteralPath $jsDistBackup -Destination $jsDist
    }
    $script:jsDistPrepared = $false
}

function Stop-PythonCoverage {
    if ($pythonStartup -and (Test-Path -LiteralPath $pythonStartup)) {
        Remove-Item -LiteralPath $pythonStartup -Force
    }
    Remove-Item Env:COVERAGE_PROCESS_START -ErrorAction SilentlyContinue
}

function Restore-ProcessState {
    Get-ChildItem Env: | ForEach-Object {
        if (-not $originalEnvironment.ContainsKey($_.Name)) {
            [Environment]::SetEnvironmentVariable($_.Name, $null, "Process")
        }
    }
    foreach ($entry in $originalEnvironment.GetEnumerator()) {
        [Environment]::SetEnvironmentVariable(
            $entry.Key,
            [string]$entry.Value,
            "Process"
        )
    }
    Set-Location $originalLocation
}

function Test-LcovSourceCovered {
    param(
        [string]$Lcov,
        [string]$SourcePattern
    )
    foreach ($record in ($Lcov -split "(?m)^end_of_record\r?\n?")) {
        if (
            $record -match "(?m)^SF:.*$SourcePattern.*$" -and
            $record -match "(?m)^LH:(\d+)$" -and
            [int]$matches[1] -gt 0
        ) {
            return $true
        }
    }
    return $false
}

function Write-JavaScriptCoverageReport {
    param(
        [string]$Name,
        [string]$ReportDirectory,
        [string[]]$Includes,
        [string]$RequiredSourcePattern
    )
    $c8 = Join-Path $root "bindings\js\node_modules\.bin\c8.cmd"
    $includeArgs = @()
    foreach ($include in $Includes) {
        $includeArgs += @("--include", $include)
    }
    New-LiteralDirectory $ReportDirectory
    Invoke-Step "$Name coverage reports" {
        & $c8 report `
            --temp-directory $jsTemp `
            --reports-dir $ReportDirectory `
            --reporter text-summary `
            --reporter html `
            --reporter lcov `
            --reporter json-summary `
            --all `
            @includeArgs `
            --exclude "node_modules/**"
    }

    $lcovPath = Join-Path $ReportDirectory "lcov.info"
    $summaryPath = Join-Path $ReportDirectory "coverage-summary.json"
    $lcov = Get-Content -LiteralPath $lcovPath -Raw
    if (-not (Test-LcovSourceCovered $lcov $RequiredSourcePattern)) {
        throw "$Name coverage did not execute its required source family"
    }
    $totals = (Get-Content -LiteralPath $summaryPath -Raw | ConvertFrom-Json).total
    if ($totals.lines.total -eq 0 -or $totals.lines.covered -eq 0) {
        throw "$Name coverage did not execute any source lines"
    }
}

function Write-Reports {
    Stop-PythonCoverage
    Remove-Item Env:NODE_V8_COVERAGE -ErrorAction SilentlyContinue

    $profileDirectory = Split-Path -Parent $env:LLVM_PROFILE_FILE
    $profiles = @(
        Get-ChildItem -LiteralPath $profileDirectory -Filter "*.profraw" -File -ErrorAction SilentlyContinue
    )
    if ($profiles.Count -gt 0) {
        New-LiteralDirectory @($rustReport, $rustRaw)
        Copy-Item -LiteralPath $profiles.FullName -Destination $rustRaw -Force

        Invoke-Step "Rust HTML coverage report" {
            & cargo llvm-cov report --profile coverage --target $script:cargoTarget --failure-mode any `
                --html --output-dir $rustReport
        }
        Invoke-Step "Rust LCOV coverage report" {
            & cargo llvm-cov report --profile coverage --target $script:cargoTarget --failure-mode any `
                --lcov --output-path (Join-Path $rustReport "lcov.info")
        }
        Invoke-Step "Rust Cobertura coverage report" {
            & cargo llvm-cov report --profile coverage --target $script:cargoTarget --failure-mode any `
                --cobertura --output-path (Join-Path $rustReport "cobertura.xml")
        }
        Invoke-Step "Rust JSON coverage summary" {
            & cargo llvm-cov report --profile coverage --target $script:cargoTarget --failure-mode any `
                --json --summary-only --output-path (Join-Path $rustReport "summary.json")
        }

        $lcov = Get-Content -LiteralPath (Join-Path $rustReport "lcov.info") -Raw
        foreach ($requiredSource in @(
            "bindings/js/src/lib.rs",
            "bindings/py/src/runtime.rs"
        )) {
            $pattern = [regex]::Escape($requiredSource).Replace("/", "[\\/]")
            if ($lcov -notmatch $pattern) {
                throw "Rust coverage did not include native binding source: $requiredSource"
            }
        }
    } else {
        throw "No Rust .profraw files were produced"
    }

    $pythonData = @(Get-ChildItem -LiteralPath $pythonReport -Filter ".coverage.*" -File -ErrorAction SilentlyContinue)
    if ($pythonData.Count -gt 0) {
        Invoke-Step "Combine Python coverage" {
            & $script:pythonExe -m coverage combine --keep $pythonReport
        }
        $allPythonData = Join-Path $pythonReport "all-data.json"
        & $script:pythonExe -m coverage json --ignore-errors -o $allPythonData
        if ($LASTEXITCODE -ne 0) {
            throw "Inspecting Python coverage data failed"
        }
        $coverageJson = Get-Content -LiteralPath $allPythonData -Raw | ConvertFrom-Json -AsHashtable
        $measuredFiles = @($coverageJson["files"].Keys)
        $productFiles = @($measuredFiles | Where-Object {
            $_ -match "tests[\\/]e2e[\\/]e2e_generated[\\/]python_bindings" -or
            ($_ -match "dynwinrt" -and $_ -notmatch "bindings[\\/]py[\\/]tests")
        })
        if ($productFiles.Count -gt 0) {
            Invoke-Step "Python HTML coverage report" {
                & $script:pythonExe -m coverage html --rcfile $pythonConfig `
                    -d (Join-Path $pythonReport "html")
            }
            Invoke-Step "Python XML coverage report" {
                & $script:pythonExe -m coverage xml --rcfile $pythonConfig `
                    -o (Join-Path $pythonReport "coverage.xml")
            }
            Invoke-Step "Python LCOV coverage report" {
                & $script:pythonExe -m coverage lcov --rcfile $pythonConfig `
                    -o (Join-Path $pythonReport "lcov.info")
            }
            Invoke-Step "Python JSON coverage summary" {
                & $script:pythonExe -m coverage json --rcfile $pythonConfig `
                    -o (Join-Path $pythonReport "coverage.json")
            }
            & $script:pythonExe -m coverage report --rcfile $pythonConfig
        } else {
            if ($SkipE2E) {
                Write-Warning "No generated Python projection files were measured; skipping Python reports"
            } else {
                throw "No generated Python projection files were measured"
            }
        }
    } else {
        throw "No Python coverage data was produced"
    }

    $jsData = @(Get-ChildItem -LiteralPath $jsTemp -Filter "*.json" -File -ErrorAction SilentlyContinue)
    if ($jsData.Count -gt 0) {
        $aggregateIncludes = @("bindings/js/dist/**/*.js")
        if ($winrtCoverageExpected) {
            $aggregateIncludes += "tests/e2e/e2e_generated/ts/**/*.js"
        }
        if ($comCoverageExpected) {
            $aggregateIncludes += "tests/e2e/e2e_generated/com/**/*.js"
        }

        Write-JavaScriptCoverageReport `
            -Name "JavaScript aggregate" `
            -ReportDirectory $jsReport `
            -Includes $aggregateIncludes `
            -RequiredSourcePattern "bindings[\\/]js[\\/]dist[\\/]index\.js"
        Write-JavaScriptCoverageReport `
            -Name "JavaScript runtime" `
            -ReportDirectory $jsRuntimeReport `
            -Includes @("bindings/js/dist/**/*.js") `
            -RequiredSourcePattern "bindings[\\/]js[\\/]dist[\\/]index\.js"
        if ($winrtCoverageExpected) {
            Write-JavaScriptCoverageReport `
                -Name "Generated WinRT" `
                -ReportDirectory $jsWinrtReport `
                -Includes @("tests/e2e/e2e_generated/ts/**/*.js") `
                -RequiredSourcePattern "tests[\\/]e2e[\\/]e2e_generated[\\/]ts[\\/]"
        }
        if ($comCoverageExpected) {
            Write-JavaScriptCoverageReport `
                -Name "Generated Classic COM" `
                -ReportDirectory $jsComReport `
                -Includes @("tests/e2e/e2e_generated/com/**/*.js") `
                -RequiredSourcePattern "tests[\\/]e2e[\\/]e2e_generated[\\/]com[\\/]"
        }
    } else {
        throw "No V8 coverage data was produced"
    }
}

function Get-CoverageSummaryMarkdown {
    $rows = @()

    $rustSummary = Join-Path $rustReport "summary.json"
    if (Test-Path -LiteralPath $rustSummary) {
        $totals = (Get-Content -LiteralPath $rustSummary -Raw | ConvertFrom-Json).data[0].totals
        Assert-MinimumCoverage "Rust" $totals.lines.percent $MinRustLineCoverage
        $rows += "| Rust, including native .pyd/.node | $([math]::Round($totals.lines.percent, 2))% | $([math]::Round($totals.functions.percent, 2))% | $([math]::Round($totals.regions.percent, 2))% regions |"
    }

    $pythonSummary = Join-Path $pythonReport "coverage.json"
    if (Test-Path -LiteralPath $pythonSummary) {
        $totals = (Get-Content -LiteralPath $pythonSummary -Raw | ConvertFrom-Json -AsHashtable)["totals"]
        $linePercent = if ($totals["num_statements"]) {
            [math]::Round(100 * $totals["covered_lines"] / $totals["num_statements"], 2)
        } else {
            100
        }
        $branchPercent = if ($totals["num_branches"]) {
            [math]::Round(100 * $totals["covered_branches"] / $totals["num_branches"], 2)
        } else {
            100
        }
        Assert-MinimumCoverage "Generated Python" $linePercent $MinPythonLineCoverage
        $rows += "| Generated Python projections | $linePercent% | n/a | $branchPercent% branches |"
    }

    foreach ($javascriptLayer in @(
        [pscustomobject]@{ Name = "JavaScript aggregate"; Path = $jsReport }
        [pscustomobject]@{ Name = "JavaScript runtime"; Path = $jsRuntimeReport }
        [pscustomobject]@{ Name = "Generated WinRT projections"; Path = $jsWinrtReport }
        [pscustomobject]@{ Name = "Generated Classic COM projections"; Path = $jsComReport }
    )) {
        $jsSummary = Join-Path $javascriptLayer.Path "coverage-summary.json"
        if (Test-Path -LiteralPath $jsSummary) {
            $totals = (Get-Content -LiteralPath $jsSummary -Raw | ConvertFrom-Json).total
            if ($javascriptLayer.Name -eq "JavaScript aggregate") {
                Assert-MinimumCoverage `
                    $javascriptLayer.Name `
                    $totals.lines.pct `
                    $MinJavaScriptLineCoverage
            }
            $rows += "| $($javascriptLayer.Name) | $($totals.lines.pct)% | $($totals.functions.pct)% | $($totals.branches.pct)% branches |"
        }
    }

    if ($rows.Count -gt 0) {
        return @(
            "# Mixed-language coverage"
            ""
            "| Layer | Lines | Functions | Branches/regions |"
            "| --- | ---: | ---: | ---: |"
        ) + $rows
    }

    return @()
}

function Write-CoverageSummary {
    $summary = @(Get-CoverageSummaryMarkdown)
    if ($summary.Count -gt 0) {
        Set-Content -LiteralPath (Join-Path $output "summary.md") -Value $summary -Encoding utf8
    }
}

if ($ValidateOnly) {
    Set-Location $root
    $summary = @(Get-CoverageSummaryMarkdown)
    if ($summary.Count -eq 0) {
        throw "No coverage summaries were found under $output"
    }
    foreach ($line in $summary) {
        Write-Host $line
    }
    return
}

$scriptError = $null
try {
    Set-Location $root
    if (-not $SkipE2E) {
        $e2eGeneratedByCoverage = $true
        if (Test-Path -LiteralPath $e2eGenerated) {
            Remove-Item -LiteralPath $e2eGenerated -Recurse -Force
        }
    }
    if (Test-Path -LiteralPath $output -PathType Leaf) {
        throw "OutputDirectory must be a directory: $output"
    }
    $hasValidOutputMarker = $false
    if (Test-Path -LiteralPath $outputMarker) {
        $marker = Get-Item -LiteralPath $outputMarker -Force
        if (
            $marker.PSIsContainer -or
            (($marker.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0)
        ) {
            throw "Coverage output marker must be a regular file: $outputMarker"
        }
        $markerContent = [string](Get-Content -LiteralPath $outputMarker -Raw)
        $hasValidOutputMarker = $markerContent.Trim() -eq $outputMarkerText
    }
    if (Test-Path -LiteralPath $output) {
        $existingOutput = @(Get-ChildItem -LiteralPath $output -Force)
        if (
            $existingOutput.Count -gt 0 -and
            -not $hasValidOutputMarker -and
            -not (Test-PathContains $repositoryArtifactRoot $output)
        ) {
            throw "Refusing to clean a non-empty, unmanaged output directory: $output"
        }
    } else {
        New-LiteralDirectory $output
    }
    Set-Content -LiteralPath $outputMarker -Value $outputMarkerText -Encoding ascii
    foreach ($ownedOutput in @(
        $rustReport,
        $pythonReport,
        $jsReport,
        (Join-Path $output "raw"),
        (Join-Path $output "summary.md")
    )) {
        if (Test-Path -LiteralPath $ownedOutput) {
            Assert-NoReparsePointTree $ownedOutput
            Remove-Item -LiteralPath $ownedOutput -Recurse -Force
        }
    }
    New-LiteralDirectory @($rustReport, $rustRaw, $pythonReport, $jsTemp)
    $env:LLVM_PROFILE_FILE = Join-Path $rustRaw "dynwinrt-%p-%m.profraw"

    $basePython = Resolve-Python
    New-LiteralDirectory $scratchRoot
    Invoke-Step "Create isolated Python coverage environment" {
        & $basePython -m venv $pythonEnvironment
    }
    $script:pythonExe = (Resolve-Path -LiteralPath (Join-Path $pythonEnvironment "Scripts\python.exe")).Path
    $env:VIRTUAL_ENV = $pythonEnvironment
    $env:PATH = "$(Join-Path $pythonEnvironment 'Scripts');$env:PATH"
    $script:cargoTarget = if ($CargoTarget) {
        $CargoTarget
    } else {
        $hostLine = & rustc -vV | Where-Object { $_ -like "host:*" }
        if (-not $hostLine) { throw "Could not determine the Rust host target" }
        ($hostLine -split ":", 2)[1].Trim()
    }

    try {
        if (-not (Get-Command cargo-llvm-cov -ErrorAction SilentlyContinue)) {
            throw "cargo-llvm-cov 0.8.7 is required. Run: cargo install cargo-llvm-cov --version 0.8.7 --locked"
        }
        if (-not (Get-Command node -ErrorAction SilentlyContinue)) {
            throw "Node.js is required"
        }

        Invoke-Step "Upgrade isolated Python pip" {
            & $script:pythonExe -m pip install "pip>=25.1,<27" --quiet
        }
        Invoke-Step "Install Python coverage dependencies" {
            & $script:pythonExe -m pip install `
                "coverage[toml]>=7.15,<8" `
                "maturin>=1.11,<2" `
                "mypy>=1.13,<2" `
                "pytest>=8.3.5" `
                --quiet
        }
        Invoke-Step "Install JavaScript dependencies" {
            Push-Location (Join-Path $root "bindings\js")
            try {
                & npm ci --no-audit --no-fund
            } finally {
                Pop-Location
            }
        }

        if ($Win32Winmd) {
            $resolvedWinmd = (Resolve-Path -LiteralPath $Win32Winmd).Path
            $env:DYNWINRT_WIN32_WINMD = $resolvedWinmd
            $env:DYNWINRT_REQUIRE_WIN32_METADATA = "1"
        }

        $env:CARGO_INCREMENTAL = "0"
        $env:LLVM_PROFILE_FILE_NAME = "dynwinrt-%p-%m.profraw"
        $coverageEnvironment = (
            & cargo llvm-cov show-env --pwsh --target $script:cargoTarget | Out-String
        )
        if ($LASTEXITCODE -ne 0) {
            throw "cargo llvm-cov show-env failed"
        }
        Invoke-Expression $coverageEnvironment

        Invoke-Step "Clean previous Rust coverage data" {
            & cargo llvm-cov clean --workspace
        }
        Invoke-Step "Rust workspace tests" {
            & cargo test `
                -p dynwinrt `
                -p dynwinrt-codegen `
                -p dynwinrt-py `
                -p jswinrt_rs `
                --profile coverage `
                --target $script:cargoTarget
        }
        Invoke-Step "Build instrumented Python binding" {
            Push-Location (Join-Path $root "bindings\py")
            try {
                & $script:pythonExe -m maturin develop --profile coverage `
                    --target $script:cargoTarget
            } finally {
                Pop-Location
            }
        }
        Invoke-Step "Build instrumented JavaScript binding" {
            Prepare-JavaScriptOutput
            Push-Location (Join-Path $root "bindings\js")
            try {
                $napi = Join-Path $root "bindings\js\node_modules\.bin\napi.cmd"
                if (-not (Test-Path -LiteralPath $napi)) {
                    throw "NAPI CLI is missing: $napi"
                }
                & $napi build --no-const-enum --profile coverage --platform `
                    --target $script:cargoTarget -o dist
                if ($LASTEXITCODE -ne 0) { return }
                & npm run build:entrypoints --silent
            } finally {
                Pop-Location
            }
        }

        $env:COVERAGE_FILE = Join-Path $pythonReport ".coverage"
        & $script:pythonExe -m coverage erase
        if ($LASTEXITCODE -ne 0) {
            throw "coverage erase failed"
        }
        $env:COVERAGE_PROCESS_START = $pythonConfig
        $sitePackages = (
            & $script:pythonExe -c "import sysconfig; print(sysconfig.get_path('purelib'))"
        ).Trim()
        $pythonStartup = Join-Path $sitePackages "dynwinrt_coverage.pth"
        Set-Content -LiteralPath $pythonStartup -Value "import coverage; coverage.process_startup()" -Encoding ascii

        $env:NODE_V8_COVERAGE = $jsTemp

        Invoke-Step "Python binding tests" {
            & $script:pythonExe -m pytest bindings\py\tests -q
        }
        Invoke-Step "Python stubtest" {
            & $script:pythonExe -m mypy.stubtest dynwinrt `
                --allowlist bindings\py\stubtest_allowlist.txt `
                --ignore-disjoint-bases
        }
        Invoke-Step "JavaScript binding tests" {
            Push-Location (Join-Path $root "bindings\js")
            try {
                & npm test -- --no-color
            } finally {
                Pop-Location
            }
        }

        if (-not $SkipE2E) {
            $winrtCoverageExpected = $true
            $languages = @("py", "ts")
            if (-not $SkipCom -and $env:DYNWINRT_WIN32_WINMD) {
                $languages += "com"
                $comCoverageExpected = $true
            }
            Invoke-Step "Cross-language E2E tests" {
                & (Join-Path $root "tests\e2e\e2e_test.ps1") `
                    -SkipBuild `
                    -KeepGenerated `
                    -CargoProfile coverage `
                    -CargoTarget $script:cargoTarget `
                    -Python $script:pythonExe `
                    -Lang $languages
            }
        }
    } catch {
        $pipelineError = $_
        Write-Warning "Coverage test pipeline failed: $($_.Exception.Message)"
    } finally {
        try {
            Write-Reports
        } catch {
            if (-not $pipelineError) {
                $pipelineError = $_
            } else {
                Write-Warning "Coverage report generation also failed: $($_.Exception.Message)"
            }
        }
        try {
            Stop-PythonCoverage
        } catch {
            if (-not $pipelineError) {
                $pipelineError = $_
            } else {
                Write-Warning "Coverage cleanup also failed: $($_.Exception.Message)"
            }
        }
        try {
            Write-CoverageSummary
        } catch {
            if (-not $pipelineError) {
                $pipelineError = $_
            } else {
                Write-Warning "Coverage summary generation also failed: $($_.Exception.Message)"
            }
        }
    }

    if ($pipelineError) {
        throw $pipelineError
    }

    Write-Host "`nCoverage reports:" -ForegroundColor Green
    Write-Host "  Rust:       $(Join-Path $rustReport 'html\index.html')"
    if (Test-Path -LiteralPath (Join-Path $pythonReport "html\index.html")) {
        Write-Host "  Python:     $(Join-Path $pythonReport 'html\index.html')"
    }
    Write-Host "  JavaScript: $(Join-Path $jsReport 'index.html')"
    Write-Host "    Runtime:  $(Join-Path $jsRuntimeReport 'index.html')"
    if ($winrtCoverageExpected) {
        Write-Host "    WinRT:    $(Join-Path $jsWinrtReport 'index.html')"
    }
    if ($comCoverageExpected) {
        Write-Host "    COM:      $(Join-Path $jsComReport 'index.html')"
    }
} catch {
    $scriptError = $_
} finally {
    try {
        Restore-JavaScriptOutput
    } catch {
        if (-not $scriptError) {
            $scriptError = $_
        } else {
            Write-Warning "Restoring the JavaScript output also failed: $($_.Exception.Message)"
        }
    }
    try {
        if ($e2eGeneratedByCoverage -and (Test-Path -LiteralPath $e2eGenerated)) {
            Remove-Item -LiteralPath $e2eGenerated -Recurse -Force
        }
        if (Test-Path -LiteralPath $scratchRoot) {
            Remove-Item -LiteralPath $scratchRoot -Recurse -Force
        }
    } catch {
        if (-not $scriptError) {
            $scriptError = $_
        } else {
            Write-Warning "Removing temporary coverage files also failed: $($_.Exception.Message)"
        }
    }
    try {
        Restore-ProcessState
    } catch {
        if (-not $scriptError) {
            $scriptError = $_
        } else {
            Write-Warning "Restoring the caller process state also failed: $($_.Exception.Message)"
        }
    }
}

if ($scriptError) {
    throw $scriptError
}
