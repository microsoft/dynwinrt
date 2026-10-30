# Copyright (c) Microsoft Corporation.
# Licensed under the MIT License.

param(
    [Parameter(Mandatory = $true)][string]$Python,
    [Parameter(Mandatory = $true)][string]$RuntimeWheelDirectory,
    [Parameter(Mandatory = $true)][string]$CodegenWheelDirectory,
    [Parameter(Mandatory = $true)][string]$Version,
    [Parameter(Mandatory = $true)][string]$PythonPackageVersion,
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
    --find-links $RuntimeWheelDirectory "dynwinrt==$Version"
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
& $venvPython -m pip install --disable-pip-version-check --no-index `
    --find-links $CodegenWheelDirectory "dynwinrt-codegen==$Version"
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
& $venvPython -m pip install --disable-pip-version-check "pytest>=8.3.5" --quiet
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
& $venvPython -m pytest (Join-Path $repoRoot 'bindings\py\tests\test_async_event_scope.py') `
    (Join-Path $repoRoot 'bindings\py\tests\test_winappsdk.py') -q
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

$native = @(Get-ChildItem (Join-Path $root 'venv\Lib\site-packages\dynwinrt') -Filter *.pyd -File)
if ($native.Count -ne 1) { throw 'Expected exactly one installed production Python extension' }
& (Get-Command node -ErrorAction Stop).Source `
    (Join-Path $repoRoot 'bindings\js\scripts\check-ui-helper-imports.mjs') $native[0].FullName
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

$scripts = Join-Path $root 'venv\Scripts'
$env:PATH = "$scripts;$env:SystemRoot\System32;$env:SystemRoot"
$env:PYTHONPATH = ''
$codegen = Join-Path $scripts 'dynwinrt-codegen.exe'
$generated = Join-Path $root 'generated_uri'
& $codegen generate `
    --class-name "Windows.Foundation.Uri,Windows.Foundation.Collections.PropertySet,Windows.Storage.StorageFolder" `
    --lang py --output $generated
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

$generatedInstall = Join-Path $root 'consumer'
New-Item $generatedInstall -ItemType Directory -Force | Out-Null
& $venvPython -m pip install --disable-pip-version-check --no-deps $generated
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
assert importlib.metadata.version("dynwinrt") == expected_version
assert importlib.metadata.version("dynwinrt-codegen") == expected_version

import dynwinrt
from dynwinrt import RoApartment, projected_lifetime_scope
from generated_uri import Uri

assert ".python-release" in str(pathlib.Path(dynwinrt.__file__).resolve())
with RoApartment(1), projected_lifetime_scope():
    uri = Uri("https://example.com:443/release?q=1")
    assert uri.host == "example.com"
    assert uri.scheme_name == "https"
    assert uri.port == 443
print("isolated runtime consumer OK")
'@

Push-Location $generatedInstall
try {
    & $venvPython -c $test $Architecture $PythonMinor $PythonPackageVersion
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
    & $venvPython -m pip install --disable-pip-version-check --no-deps --editable $generated
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
    & $venvPython -c $test $Architecture $PythonMinor $PythonPackageVersion
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
    $pth = @(Get-ChildItem (Join-Path $root 'venv\Lib\site-packages') -Filter '*generated_uri*.pth')
    if ($pth.Count -ne 1 -or (Get-Content $pth[0].FullName -Raw).Trim() -ne (Join-Path $generated 'src')) {
        throw 'Generated Python editable install must use a plain src path .pth'
    }
    foreach ($mode in @(0, 1)) {
        & $venvPython -I (Join-Path $repoRoot 'tests\e2e\runners\py_async_events.py') `
            --generated $generated --mode $mode
        if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
    }
} finally {
    Pop-Location
}
