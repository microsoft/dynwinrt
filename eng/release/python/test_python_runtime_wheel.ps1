# Copyright (c) Microsoft Corporation.
# Licensed under the MIT License.

param(
    [Parameter(Mandatory = $true)][string]$Python,
    [Parameter(Mandatory = $true)][string]$RuntimeWheelDirectory,
    [Parameter(Mandatory = $true)][string]$CodegenWheelDirectory,
    [Parameter(Mandatory = $true)][string]$Version,
    [Parameter(Mandatory = $true)][ValidateSet('x64', 'arm64')][string]$Architecture,
    [Parameter(Mandatory = $true)][string]$PythonMinor
)

$ErrorActionPreference = 'Stop'
$label = $PythonMinor.Replace('.', '')
$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot '..\..\..')).Path
$root = Join-Path $repoRoot ".python-release\runtime-$Architecture-cp$label"
Remove-Item $root -Recurse -Force -ErrorAction SilentlyContinue
New-Item $root -ItemType Directory -Force | Out-Null

& $Python -m venv (Join-Path $root 'venv')
$venvPython = Join-Path $root 'venv\Scripts\python.exe'
& $venvPython -m pip install --disable-pip-version-check --no-index `
    --find-links $RuntimeWheelDirectory "dynwinrt-py==$Version"
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
& $venvPython -m pip install --disable-pip-version-check --no-index `
    --find-links $CodegenWheelDirectory "dynwinrt-codegen==$Version"
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

$scripts = Join-Path $root 'venv\Scripts'
$env:PATH = "$scripts;$env:SystemRoot\System32;$env:SystemRoot"
$env:PYTHONPATH = ''
$codegen = Join-Path $scripts 'dynwinrt-codegen.exe'
$generated = Join-Path $root 'generated_uri'
& $codegen generate --namespace Windows.Foundation --class-name Uri `
    --lang py --output $generated
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

$test = @'
import importlib.metadata
import pathlib
import platform
import sys

expected_arch, expected_minor, expected_version = sys.argv[1:4]
assert platform.machine().lower() in (
    ("amd64", "x86_64") if expected_arch == "x64" else ("arm64", "aarch64")
)
assert f"{sys.version_info.major}.{sys.version_info.minor}" == expected_minor
assert importlib.metadata.version("dynwinrt-py") == expected_version
assert importlib.metadata.version("dynwinrt-codegen") == expected_version

import dynwinrt_py
from dynwinrt_py import RoApartment, projected_lifetime_scope
from generated_uri import Uri

assert ".python-release" in str(pathlib.Path(dynwinrt_py.__file__).resolve())
with RoApartment(1), projected_lifetime_scope():
    uri = Uri("https://example.com:443/release?q=1")
    assert uri.host == "example.com"
    assert uri.scheme_name == "https"
    assert uri.port == 443
print("isolated runtime consumer OK")
'@

Push-Location $root
try {
    & $venvPython -c $test $Architecture $PythonMinor $Version
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
} finally {
    Pop-Location
}
