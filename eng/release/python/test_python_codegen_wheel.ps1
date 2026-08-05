# Copyright (c) Microsoft Corporation.
# Licensed under the MIT License.

param(
    [Parameter(Mandatory = $true)][string]$Python,
    [Parameter(Mandatory = $true)][string]$WheelDirectory,
    [Parameter(Mandatory = $true)][string]$Version,
    [Parameter(Mandatory = $true)][ValidateSet('x64', 'arm64')][string]$Architecture
)

$ErrorActionPreference = 'Stop'
$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot '..\..\..')).Path
$root = Join-Path $repoRoot ".python-release\codegen-$Architecture"
Remove-Item $root -Recurse -Force -ErrorAction SilentlyContinue
New-Item $root -ItemType Directory -Force | Out-Null

& $Python -m venv (Join-Path $root 'venv')
$venvPython = Join-Path $root 'venv\Scripts\python.exe'
& $venvPython -m pip install --disable-pip-version-check --no-index `
    --find-links $WheelDirectory "dynwinrt-codegen==$Version"
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

$scripts = Join-Path $root 'venv\Scripts'
$env:PATH = "$scripts;$env:SystemRoot\System32;$env:SystemRoot"
if (Get-Command cargo -ErrorAction SilentlyContinue) {
    throw 'cargo must not be visible to the codegen wheel consumer'
}
if (Get-Command rustc -ErrorAction SilentlyContinue) {
    throw 'rustc must not be visible to the codegen wheel consumer'
}

$codegen = Join-Path $scripts 'dynwinrt-codegen.exe'
$jsOutput = Join-Path $root 'generated_js'
$pyOutput = Join-Path $root 'generated_py'
& $codegen generate --namespace Windows.Foundation --class-name Uri `
    --lang js --output $jsOutput
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
& $codegen generate --namespace Windows.Foundation --class-name Uri `
    --lang py --output $pyOutput
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

if (-not (Get-ChildItem $jsOutput -Filter *.js -File -Recurse) -or
    -not (Get-ChildItem $jsOutput -Filter *.d.ts -File -Recurse)) {
    throw 'The installed codegen wheel did not generate JS and TypeScript declarations'
}
if (-not (Get-ChildItem $pyOutput -Filter *.py -File -Recurse) -or
    -not (Get-ChildItem $pyOutput -Filter *.pyi -File -Recurse) -or
    -not (Test-Path (Join-Path $pyOutput 'pyproject.toml'))) {
    throw 'The installed codegen wheel did not generate a typed Python package'
}

$manifest = Get-Content (Join-Path $pyOutput 'pyproject.toml') -Raw
if ($manifest -notmatch [regex]::Escape("dependencies = [`"dynwinrt-py==$Version`"]")) {
    throw "Generated package does not pin dynwinrt-py==$Version"
}
Write-Host "Standalone $Architecture codegen wheel consumed without cargo or rustc"
