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
$arguments = @(
    "generate",
    "--class-name",
    "Windows.ApplicationModel.Background.IBackgroundTask,Windows.ApplicationModel.Background.IBackgroundTaskInstance,Windows.Foundation.IStringable,Windows.Foundation.IClosable",
    "--lang", "py",
    "--output", (Join-Path $PSScriptRoot "generated")
)
if ($Winmd) {
    $arguments += @("--winmd", (Resolve-Path -LiteralPath $Winmd).Path)
}
& $Codegen @arguments
if ($LASTEXITCODE -ne 0) {
    throw "WinRT interface generation failed with exit code $LASTEXITCODE"
}
