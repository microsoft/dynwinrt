#!/usr/bin/env pwsh
# Copyright (c) Microsoft Corporation.
# Licensed under the MIT License.
#
# E2E test orchestrator: build, generate, run language-specific runners, collect results.
# Test logic lives in runners/py_runner.py, runners/ts_runner.ts, and runners/com/*.mjs.
#
# Usage:
#   .\tests\e2e\e2e_test.ps1                    # Full (build + generate + test)
#   .\tests\e2e\e2e_test.ps1 -SkipBuild         # Skip build step
#   .\tests\e2e\e2e_test.ps1 -Lang py           # Python only
#   .\tests\e2e\e2e_test.ps1 -Lang ts           # TypeScript only
#   .\tests\e2e\e2e_test.ps1 -Lang com          # Classic COM only
#   .\tests\e2e\e2e_test.ps1 -Lang win32        # Flat Win32 only

param(
    [switch]$SkipBuild,
    [switch]$KeepGenerated,
    [string]$CargoProfile = "release",
    [string]$CargoTarget,
    [string]$Python,
    [ValidateSet("py", "ts", "com", "win32")]
    [string[]]$Lang = @("py", "ts", "com", "win32")
)

$ErrorActionPreference = "Stop"
$langWasExplicit = $PSBoundParameters.ContainsKey("Lang")
$root = Split-Path (Split-Path $PSScriptRoot -Parent) -Parent
$specsFile = Join-Path $PSScriptRoot "e2e_specs.json"
$e2eDir = Join-Path $PSScriptRoot "e2e_generated"
$runnersDir = Join-Path $PSScriptRoot "runners"
$pyBindingsDir = Join-Path $e2eDir "python_bindings"
$comBindingsDir = Join-Path $e2eDir "com"
$comShellDir = Join-Path $comBindingsDir "shell"
$comInteropDir = Join-Path $comBindingsDir "interop"
$comWicDir = Join-Path $comBindingsDir "wic"
$comStreamDir = Join-Path $comBindingsDir "stream"
$comAutomationDir = Join-Path $comBindingsDir "automation"
$comInfrastructureDir = Join-Path $comBindingsDir "infrastructure"
$comSmtcDir = Join-Path $comBindingsDir "smtc"
$win32BindingsDir = Join-Path $e2eDir "win32"
[string[]]$cargoProfileArgs = @(
    if ($CargoProfile -eq "release") {
        "--release"
    } else {
        "--profile"
        $CargoProfile
    }
)
[string[]]$cargoTargetArgs = @(
    if ($CargoTarget) {
        "--target"
        $CargoTarget
    }
)

$env:PATH = "$env:USERPROFILE\.cargo\bin;$env:PATH"

Write-Host "=== dynwinrt E2E Test ===" -ForegroundColor Cyan

# --------------------------------------------------------------------------
# Detect available tools
# --------------------------------------------------------------------------
$pythonExe = if ($Python) {
    (Resolve-Path -LiteralPath $Python).Path
} else {
    (Get-Command python -ErrorAction SilentlyContinue).Source
}
$hasPython = [bool]$pythonExe
$hasNode = [bool](Get-Command node -ErrorAction SilentlyContinue)

if ("py" -in $Lang -and -not $hasPython) {
    Write-Host "  SKIP Python (not installed)" -ForegroundColor DarkYellow
    $Lang = $Lang | Where-Object { $_ -ne "py" }
}
if (("ts" -in $Lang -or "com" -in $Lang -or "win32" -in $Lang) -and -not $hasNode) {
    Write-Host "  SKIP JavaScript E2E (Node.js not installed)" -ForegroundColor DarkYellow
    $Lang = @($Lang | Where-Object { $_ -notin @("ts", "com", "win32") })
}

function Find-Win32Winmd {
    if ($env:DYNWINRT_WIN32_WINMD -and (Test-Path $env:DYNWINRT_WIN32_WINMD)) {
        return (Resolve-Path -LiteralPath $env:DYNWINRT_WIN32_WINMD).Path
    }

    $packageRoot = Join-Path $env:USERPROFILE ".nuget\packages\microsoft.windows.sdk.win32metadata"
    if (Test-Path $packageRoot) {
        $candidate = Get-ChildItem $packageRoot -Filter Windows.Win32.winmd -File -Recurse |
            Sort-Object LastWriteTime -Descending |
            Select-Object -First 1
        if ($candidate) { return $candidate.FullName }
    }

    $legacyPath = "C:\s\win32metadata\Windows.Win32.winmd"
    if (Test-Path $legacyPath) { return $legacyPath }
    return $null
}

$win32Winmd = $null
if ("com" -in $Lang -or "win32" -in $Lang) {
    $win32Winmd = Find-Win32Winmd
    if (-not $win32Winmd) {
        if ($langWasExplicit -or $env:DYNWINRT_REQUIRE_WIN32_METADATA -eq "1") {
            Write-Error "Classic COM and flat Win32 E2E require Windows.Win32.winmd. Set DYNWINRT_WIN32_WINMD or install Microsoft.Windows.SDK.Win32Metadata."
            exit 1
        }
        Write-Host "  SKIP Classic COM/flat Win32 (Windows.Win32.winmd not found)" -ForegroundColor DarkYellow
        $Lang = @($Lang | Where-Object { $_ -notin @("com", "win32") })
    } else {
        $env:DYNWINRT_WIN32_WINMD = $win32Winmd
        Write-Host "  Win32 metadata: $win32Winmd"
    }
}

if ($Lang.Count -eq 0) { Write-Error "No languages available"; exit 1 }

function Invoke-NodeRunner([string]$runnerPath, [int]$timeoutSeconds = 180) {
    $nodePath = (Get-Command node).Source
    $process = Start-Process -FilePath $nodePath -ArgumentList @($runnerPath) -NoNewWindow -PassThru
    if (-not $process.WaitForExit($timeoutSeconds * 1000)) {
        Write-Host "TIMEOUT: $runnerPath exceeded ${timeoutSeconds}s" -ForegroundColor Red
        Stop-Process -Id $process.Id -Force
        $process.WaitForExit()
        return 124
    }
    return $process.ExitCode
}

# --------------------------------------------------------------------------
# Build (optional)
# --------------------------------------------------------------------------
if (-not $SkipBuild) {
    Write-Host "`n--- Build ---" -ForegroundColor Yellow

    & cargo build -p dynwinrt-codegen @cargoProfileArgs @cargoTargetArgs
    if ($LASTEXITCODE -ne 0) { Write-Error "dynwinrt-codegen build failed"; exit 1 }

    if ("py" -in $Lang) {
        Push-Location (Join-Path $root "bindings\py")
        if (-not $Python) {
            $venvPython = Join-Path (Get-Location) ".venv\Scripts\python.exe"
            if (-not (Test-Path $venvPython)) {
                & $pythonExe -m venv .venv
                if ($LASTEXITCODE -ne 0) { Write-Error "Python virtual environment creation failed"; exit 1 }
                & $venvPython -m pip install pytest maturin --quiet
                if ($LASTEXITCODE -ne 0) { Write-Error "Python test dependency installation failed"; exit 1 }
            }
            $pythonExe = (Resolve-Path -LiteralPath $venvPython).Path
        }
        [string[]]$maturinProfileArgs = @(
            if ($CargoProfile -eq "release") {
                "--release"
            } else {
                "--profile"
                $CargoProfile
            }
        )
        & $pythonExe -m maturin build @maturinProfileArgs @cargoTargetArgs --quiet 2>&1 | Out-Null
        if ($LASTEXITCODE -ne 0) { Write-Error "maturin build failed"; exit 1 }
        $wheelDir = if ($env:CARGO_TARGET_DIR) { Join-Path $env:CARGO_TARGET_DIR "wheels" } else { Join-Path $root "target\wheels" }
        $whl = (Get-ChildItem (Join-Path $wheelDir "*.whl") | Sort-Object LastWriteTime -Descending | Select-Object -First 1).FullName
        if (-not $whl) { Write-Error "No wheel found after maturin build"; exit 1 }
        & $pythonExe -m pip install $whl --force-reinstall --quiet 2>&1 | Out-Null
        if ($LASTEXITCODE -ne 0) { Write-Error "pip install failed"; exit 1 }
        Pop-Location
    }

    if ("ts" -in $Lang -or "com" -in $Lang -or "win32" -in $Lang) {
        Push-Location (Join-Path $root "bindings\js")
        npm install --quiet 2>&1 | Out-Null
        if ($LASTEXITCODE -ne 0) { Write-Error "npm install failed"; exit 1 }
        $napi = Join-Path $root "bindings\js\node_modules\.bin\napi.cmd"
        if (-not (Test-Path -LiteralPath $napi)) { Write-Error "NAPI CLI is missing: $napi"; exit 1 }
        & $napi build --no-const-enum --platform @cargoProfileArgs @cargoTargetArgs -o dist 2>&1 | Out-Null
        if ($LASTEXITCODE -ne 0) { Write-Error "napi build failed"; exit 1 }
        npm run build:entrypoints --silent
        if ($LASTEXITCODE -ne 0) { Write-Error "runtime entrypoint generation failed"; exit 1 }
        Pop-Location
    }
} else {
    $venvPython = Join-Path $root "bindings\py\.venv\Scripts\python.exe"
    if (-not $Python -and (Test-Path $venvPython)) {
        $pythonExe = (Resolve-Path -LiteralPath $venvPython).Path
    }
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
    foreach ($ns in ($byNs.Keys | Sort-Object)) {
        $classes = ($byNs[$ns] | Select-Object -Unique) -join ","
        Write-Host "  $lang`: $ns [$classes]"
        & cargo run -p dynwinrt-codegen @cargoProfileArgs @cargoTargetArgs --quiet -- generate `
            --namespace $ns `
            --class-name $classes `
            --lang $codegenLang `
            --output $outDir
        if ($LASTEXITCODE -ne 0) { Write-Error "Generation failed: $ns ($lang)"; exit 1 }
    }
}

foreach ($l in @($Lang | Where-Object { $_ -in @("py", "ts") })) {
    Write-Host "`n--- Generate ($l) ---" -ForegroundColor Yellow
    $outDir = if ($l -eq "py") { $pyBindingsDir } else { Join-Path $e2eDir $l }
    Generate $l $outDir
}

if ("com" -in $Lang) {
    Write-Host "`n--- Generate (Classic COM) ---" -ForegroundColor Yellow
    $comRuntimeImport = "../../../../../../bindings/js/dist/com-unsafe.js"
    $winrtRuntimeImport = "../../../../../bindings/js/dist/winrt.js"

    & cargo run -p dynwinrt-codegen @cargoProfileArgs @cargoTargetArgs --quiet -- generate `
        --winmd $win32Winmd `
        --namespace Windows.Win32.UI.Shell `
        --class-name "TaskbarList,IShellLinkW,IDataTransferManagerInterop,FileOperation,FileOpenDialog" `
        --output $comShellDir `
        --import-name $comRuntimeImport
    if ($LASTEXITCODE -ne 0) { Write-Error "Classic COM Shell generation failed"; exit 1 }

    & cargo run -p dynwinrt-codegen @cargoProfileArgs @cargoTargetArgs --quiet -- generate `
        --winmd $win32Winmd `
        --namespace Windows.Win32.System.Com `
        --class-name IPersistFile `
        --output $comShellDir `
        --import-name $comRuntimeImport
    if ($LASTEXITCODE -ne 0) { Write-Error "Classic COM persistence generation failed"; exit 1 }

    & cargo run -p dynwinrt-codegen @cargoProfileArgs @cargoTargetArgs --quiet -- generate `
        --winmd $win32Winmd `
        --namespace Windows.Win32.UI.Shell.PropertiesSystem `
        --class-name IPropertyStore `
        --output $comShellDir `
        --import-name $comRuntimeImport
    if ($LASTEXITCODE -ne 0) { Write-Error "Classic COM property store generation failed"; exit 1 }

    & cargo run -p dynwinrt-codegen @cargoProfileArgs @cargoTargetArgs --quiet -- generate `
        --winmd $win32Winmd `
        --namespace Windows.Win32.System.WinRT `
        --class-name ISystemMediaTransportControlsInterop `
        --output $comInteropDir `
        --import-name $comRuntimeImport
    if ($LASTEXITCODE -ne 0) { Write-Error "Classic COM interop generation failed"; exit 1 }

    & cargo run -p dynwinrt-codegen @cargoProfileArgs @cargoTargetArgs --quiet -- generate `
        --winmd $win32Winmd `
        --namespace Windows.Win32.Graphics.Imaging `
        --class-name IWICImagingFactory `
        --output $comWicDir `
        --import-name $comRuntimeImport
    if ($LASTEXITCODE -ne 0) { Write-Error "Classic COM WIC generation failed"; exit 1 }

    & cargo run -p dynwinrt-codegen @cargoProfileArgs @cargoTargetArgs --quiet -- generate `
        --winmd $win32Winmd `
        --namespace Windows.Win32.System.Com `
        --class-name IStream `
        --output $comStreamDir `
        --import-name $comRuntimeImport
    if ($LASTEXITCODE -ne 0) { Write-Error "Classic COM stream generation failed"; exit 1 }

    & cargo run -p dynwinrt-codegen @cargoProfileArgs @cargoTargetArgs --quiet -- generate `
        --winmd $win32Winmd `
        --class-name "Windows.Win32.System.Com.IDispatch,Windows.Win32.System.Ole.IEnumVARIANT" `
        --output $comAutomationDir `
        --import-name $comRuntimeImport
    if ($LASTEXITCODE -ne 0) { Write-Error "Classic COM Automation generation failed"; exit 1 }

    & cargo run -p dynwinrt-codegen @cargoProfileArgs @cargoTargetArgs --quiet -- generate `
        --winmd $win32Winmd `
        --namespace Windows.Win32.System.Com `
        --class-name "IMalloc,IClassFactory,IErrorInfo" `
        --output $comInfrastructureDir `
        --import-name $comRuntimeImport
    if ($LASTEXITCODE -ne 0) { Write-Error "Classic COM infrastructure generation failed"; exit 1 }

    & cargo run -p dynwinrt-codegen @cargoProfileArgs @cargoTargetArgs --quiet -- generate `
        --winmd $win32Winmd `
        --namespace Windows.Win32.System.Ole `
        --class-name ICreateErrorInfo `
        --output $comInfrastructureDir `
        --import-name $comRuntimeImport
    if ($LASTEXITCODE -ne 0) { Write-Error "Classic COM error-info generation failed"; exit 1 }

    & cargo run -p dynwinrt-codegen @cargoProfileArgs @cargoTargetArgs --quiet -- generate `
        --namespace Windows.Media `
        --class-name SystemMediaTransportControls `
        --output $comSmtcDir `
        --import-name $winrtRuntimeImport
    if ($LASTEXITCODE -ne 0) { Write-Error "SMTC WinRT generation failed"; exit 1 }
}

if ("win32" -in $Lang) {
    Write-Host "`n--- Generate (flat Win32) ---" -ForegroundColor Yellow
    $win32RuntimeImport = "../../../../../../bindings/js/dist/win32.js"
    foreach ($ns in @(
        "Windows.Win32.System.Registry",
        "Windows.Win32.System.SystemInformation",
        "Windows.Win32.System.LibraryLoader",
        "Windows.Win32.Graphics.Gdi",
        "Windows.Win32.System.Threading",
        "Windows.Win32.System.Com",
        "Windows.Win32.Networking.Ldap",
        "Windows.Win32.NetworkManagement.IpHelper",
        "Windows.Win32.System.Pipes",
        "Windows.Win32.Storage.FileSystem"
    )) {
        & cargo run -p dynwinrt-codegen @cargoProfileArgs @cargoTargetArgs --quiet -- generate `
            --winmd $win32Winmd `
            --namespace $ns `
            --class-name Apis `
            --output $win32BindingsDir `
            --import-name $win32RuntimeImport
        if ($LASTEXITCODE -ne 0) { Write-Error "Flat Win32 generation failed: $ns"; exit 1 }
    }
}

# --------------------------------------------------------------------------
# Run language-specific runners
# --------------------------------------------------------------------------
$totalPass = 0
$totalFail = 0
$allResults = @()

if ("py" -in $Lang) {
    Write-Host "`n--- Python static type check ---" -ForegroundColor Yellow
    $previousMypyPath = $env:MYPYPATH
    try {
        $env:MYPYPATH = $e2eDir
        & $pythonExe -m mypy --strict (Join-Path $PSScriptRoot "typecheck\python_generated_api.py")
        if ($LASTEXITCODE -ne 0) { Write-Error "Python static type check failed"; exit 1 }
    } finally {
        $env:MYPYPATH = $previousMypyPath
    }

    Write-Host "`n--- Python E2E ---" -ForegroundColor Yellow
    $pyResult = Join-Path $e2eDir "results_py.json"
    & $pythonExe (Join-Path $runnersDir "py_runner.py") `
        --specs $specsFile `
        --generated $pyBindingsDir `
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
        --runtime (Join-Path $root "bindings\js\dist\winrt.js") `
        --output $tsResult
    if ($LASTEXITCODE -ne 0) { $totalFail++ } else { $totalPass++ }
    if (Test-Path $tsResult) { $allResults += (Get-Content $tsResult -Raw | ConvertFrom-Json) }
}

if ("com" -in $Lang) {
    Write-Host "`n--- Classic COM E2E ---" -ForegroundColor Yellow
    $comRunners = @(
        "pointer-reject-object.mjs",
        "module-commonjs.cjs",
        "taskbarlist.mjs",
        "electron-hwnd-buffer.mjs",
        "persist-file.mjs",
        "property-store.mjs",
        "shell-link-pod.mjs",
        "file-operation.mjs",
        "file-open-dialog.mjs",
        "wic-imaging-factory.mjs",
        "sequential-stream-buffer.mjs",
        "automation-values.mjs",
        "automation-dispatch.mjs",
        "com-infrastructure.mjs",
        "dtm.mjs",
        "smtc.mjs"
    )
    $comPassed = 0
    $comFailed = 0
    foreach ($runner in $comRunners) {
        Write-Host "  $runner"
        $runnerExitCode = Invoke-NodeRunner (Join-Path $runnersDir "com\$runner")
        if ($runnerExitCode -eq 0) {
            $comPassed++
        } else {
            $comFailed++
        }
    }
    if ($comFailed -eq 0) { $totalPass++ } else { $totalFail++ }
    $allResults += [pscustomobject]@{
        language = "com"
        passed = $comPassed
        total = $comRunners.Count
    }
}

if ("win32" -in $Lang) {
    Write-Host "`n--- Flat Win32 E2E ---" -ForegroundColor Yellow
    $win32Runners = @("registry.mjs", "returns.mjs")
    $win32Passed = 0
    $win32Failed = 0
    foreach ($runner in $win32Runners) {
        Write-Host "  $runner"
        $runnerExitCode = Invoke-NodeRunner (Join-Path $runnersDir "win32\$runner")
        if ($runnerExitCode -eq 0) {
            $win32Passed++
        } else {
            $win32Failed++
        }
    }
    if ($win32Failed -eq 0) { $totalPass++ } else { $totalFail++ }
    $allResults += [pscustomobject]@{
        language = "win32"
        passed = $win32Passed
        total = $win32Runners.Count
    }
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
    if (-not $KeepGenerated) {
        Remove-Item -Recurse -Force $e2eDir
    }
    exit 0
} else {
    Write-Host "SOME FAILED" -ForegroundColor Red
    exit 1
}
