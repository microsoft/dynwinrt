#!/usr/bin/env pwsh

$ErrorActionPreference = "Stop"
$packages = @(
    Get-AppxPackage `
        -Name "dynwinrt-python-gsmtc-sample" `
        -ErrorAction SilentlyContinue
)
foreach ($package in $packages) {
    Remove-AppxPackage -Package $package.PackageFullName
}
Write-Host "Removed the dynwinrt Python GSMTC sample identity"
