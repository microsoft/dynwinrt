# Copyright (c) Microsoft Corporation.
# Licensed under the MIT License.

param(
    [string]$Codegen = "dynwinrt-codegen",
    [string]$Winmd
)

$ErrorActionPreference = "Stop"
if (Test-Path -LiteralPath $Codegen -PathType Leaf) {
    $Codegen = (Resolve-Path -LiteralPath $Codegen).Path
} else {
    $Codegen = (Get-Command $Codegen -ErrorAction Stop).Source
}
$output = Join-Path $PSScriptRoot "generated"
$arguments = @(
    "generate",
    "--class-name",
    "Windows.ApplicationModel.Background.IBackgroundTask,Windows.ApplicationModel.Background.IBackgroundTaskInstance,Windows.Foundation.IStringable,Windows.Foundation.IClosable",
    "--lang", "py",
    "--output", $output
)
if ($Winmd) {
    $arguments += @("--winmd", (Resolve-Path -LiteralPath $Winmd).Path)
}
& $Codegen @arguments
if ($LASTEXITCODE -ne 0) {
    throw "WinRT interface generation failed with exit code $LASTEXITCODE"
}

Write-Host "Install this generated project into the sample interpreter with: python -m pip install -e `"$output`""
