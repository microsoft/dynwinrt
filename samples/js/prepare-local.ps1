#!/usr/bin/env pwsh

param(
    [ValidateSet("x64", "arm64")]
    [string]$Architecture
)

$ErrorActionPreference = "Stop"
$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..\..")).Path

if (-not $Architecture) {
    $Architecture = switch ([System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture) {
        "X64" { "x64" }
        "Arm64" { "arm64" }
        default {
            throw "Unsupported host architecture: $([System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture)"
        }
    }
}

Push-Location (Join-Path $repoRoot "bindings\js")
try {
    npm install --no-audit --no-fund
    if ($LASTEXITCODE -ne 0) {
        throw "Installing JavaScript runtime dependencies failed with exit code $LASTEXITCODE"
    }
    npm run build
    if ($LASTEXITCODE -ne 0) {
        throw "Building the JavaScript runtime failed with exit code $LASTEXITCODE"
    }
} finally {
    Pop-Location
}

$cargoArgs = @(
    "build",
    "-p", "dynwinrt-codegen",
    "--release"
)
if ($Architecture -eq "arm64") {
    $cargoArgs += @("--target", "aarch64-pc-windows-msvc")
}

Push-Location $repoRoot
try {
    & cargo @cargoArgs
    if ($LASTEXITCODE -ne 0) {
        throw "Building dynwinrt-codegen failed with exit code $LASTEXITCODE"
    }
} finally {
    Pop-Location
}

$source = if ($Architecture -eq "arm64") {
    Join-Path $repoRoot "target\aarch64-pc-windows-msvc\release\dynwinrt-codegen.exe"
} else {
    Join-Path $repoRoot "target\release\dynwinrt-codegen.exe"
}
$destination = Join-Path `
    $repoRoot `
    "tools\dynwinrt-codegen\npm\bin\$Architecture\dynwinrt-codegen.exe"
New-Item -ItemType Directory -Force -Path (Split-Path -Parent $destination) | Out-Null
Copy-Item -LiteralPath $source -Destination $destination -Force

Write-Host "Prepared local @microsoft/dynwinrt and dynwinrt-codegen packages for $Architecture"
