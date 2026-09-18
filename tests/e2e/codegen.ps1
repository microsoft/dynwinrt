# Copyright (c) Microsoft Corporation.
# Licensed under the MIT License.

function Get-CodegenInvocation {
    param(
        [string]$Codegen,
        [string]$CargoProfile = "release",
        [string]$CargoTarget
    )

    if (-not $Codegen -and $null -ne $env:DYNWINRT_CODEGEN) {
        throw "DYNWINRT_CODEGEN must name a prebuilt dynwinrt-codegen executable."
    }
    if ($Codegen) {
        if (-not (Test-Path -LiteralPath $Codegen -PathType Leaf)) {
            throw "Prebuilt dynwinrt-codegen does not exist or is not a file: $Codegen"
        }
        return @{
            Command = (Resolve-Path -LiteralPath $Codegen).Path
            Arguments = [string[]]@()
        }
    }
    return @{
        Command = "cargo"
        Arguments = [string[]]@(
            "run"; "-p"; "dynwinrt-codegen"
            if ($CargoProfile -eq "release") { "--release" }
            else { "--profile"; $CargoProfile }
            if ($CargoTarget) { "--target"; $CargoTarget }
            "--quiet"; "--"
        )
    }
}
