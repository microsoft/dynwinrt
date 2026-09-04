#!/usr/bin/env pwsh

param(
    [string]$Python = "python",
    [string]$Image,
    [string[]]$Expect
)

$ErrorActionPreference = "Stop"
$expectWasExplicit = $PSBoundParameters.ContainsKey("Expect")
if (-not (Test-Path -LiteralPath (Join-Path $PSScriptRoot "generated\__init__.py"))) {
    throw "Generated bindings were not found. Run generate.ps1 first."
}
if (-not $Image) {
    $Image = Join-Path $PSScriptRoot "sample.png"
    & (Join-Path $PSScriptRoot "make-test-image.ps1") -Output $Image
    if (-not $expectWasExplicit) {
        $Expect = @("DYNWINRT", "OCR", "42")
    }
}
$Image = [System.IO.Path]::GetFullPath($Image)
if (-not (Test-Path -LiteralPath $Image -PathType Leaf)) {
    throw "Image was not found: $Image"
}
if (-not [System.IO.Path]::IsPathRooted($Python)) {
    $callerPython = Join-Path (Get-Location).Path $Python
    if (Test-Path -LiteralPath $callerPython -PathType Leaf) {
        $Python = [System.IO.Path]::GetFullPath($callerPython)
    }
}

$arguments = @((Join-Path $PSScriptRoot "app.py"), "--image", $Image)
if ($Expect.Count -gt 0) {
    $arguments += "--expect"
    $arguments += $Expect
}
& $Python @arguments
if ($LASTEXITCODE -ne 0) {
    exit $LASTEXITCODE
}
