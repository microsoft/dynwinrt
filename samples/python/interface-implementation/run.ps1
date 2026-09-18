# Copyright (c) Microsoft Corporation.
# Licensed under the MIT License.

param([string]$Python = "python")

$ErrorActionPreference = "Stop"
if (-not (Test-Path -LiteralPath (Join-Path $PSScriptRoot "generated\__init__.py"))) {
    throw "Generated bindings were not found. Run generate.ps1 first."
}
if (Test-Path -LiteralPath $Python -PathType Leaf) {
    $Python = (Resolve-Path -LiteralPath $Python).Path
}
& $Python (Join-Path $PSScriptRoot "app.py")
if ($LASTEXITCODE -ne 0) {
    exit $LASTEXITCODE
}
