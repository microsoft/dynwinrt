# Copyright (c) Microsoft Corporation.
# Licensed under the MIT License.

param(
    [Parameter(Mandatory)][ValidateSet("Write", "Verify")][string]$Mode,
    [Parameter(Mandatory)][string]$Path,
    [Parameter(Mandatory)][ValidateSet("codegen", "js", "python")][string]$Kind,
    [Parameter(Mandatory)][ValidateSet("x64", "arm64")][string]$Architecture,
    [ValidateSet("release", "dev")][string]$Profile = "release",
    [ValidateSet("production", "test-hooks")][string]$Features = "production",
    [string]$PythonAbi
)

$ErrorActionPreference = "Stop"
$repo = (Resolve-Path (Join-Path $PSScriptRoot "..\..")).Path
if (-not $env:GITHUB_SHA -or -not $env:GITHUB_RUN_ID) {
    throw "Artifact identity requires GITHUB_SHA and GITHUB_RUN_ID."
}
$payload = (Get-Item -LiteralPath (Join-Path $Path "payload")).FullName
if (-not (Test-Path -LiteralPath $payload -PathType Container)) {
    throw "Missing artifact payload directory: $payload"
}
$manifestPath = Join-Path $Path "manifest.json"
$version = if ($Kind -eq "js") {
    (Get-Content -LiteralPath (Join-Path $repo "bindings\js\package.json") -Raw | ConvertFrom-Json).version
} else {
    $cargo = if ($Kind -eq "codegen") { "tools\dynwinrt-codegen\Cargo.toml" } else { "bindings\py\Cargo.toml" }
    $content = Get-Content -LiteralPath (Join-Path $repo $cargo) -Raw
    $package = [regex]::Match($content, '(?ms)^\[package\]\r?\n(.*?)(?=^\[|\z)').Groups[1].Value
    $versions = [regex]::Matches($package, '(?m)^version = "([^"]+)"\r?$')
    if ($versions.Count -ne 1) { throw "Expected one package version in $cargo" }
    $versions[0].Groups[1].Value
}
$target = if ($Architecture -eq "x64") { "x86_64-pc-windows-msvc" } else { "aarch64-pc-windows-msvc" }
$identity = [ordered]@{
    schema = 1
    commit = $env:GITHUB_SHA
    run = $env:GITHUB_RUN_ID
    kind = $Kind
    version = $version
    target = $target
    profile = $Profile
    features = $Features
    pythonAbi = $PythonAbi
}
$files = [ordered]@{}
foreach ($file in (Get-ChildItem -LiteralPath $payload -File -Recurse | Sort-Object FullName)) {
    $relative = [IO.Path]::GetRelativePath($payload, $file.FullName).Replace("\", "/")
    $files[$relative] = (Get-FileHash -LiteralPath $file.FullName -Algorithm SHA256).Hash
}
if ($files.Count -eq 0) { throw "Artifact payload is empty: $payload" }

if ($Kind -eq "python") {
    if ($Features -ne "production" -or $PythonAbi -notmatch '^cp3\d+$') {
        throw "Python artifacts require a production wheel and an explicit CPython ABI."
    }
    $platform = if ($Architecture -eq "x64") { "win_amd64" } else { "win_arm64" }
    $wheel = "dynwinrt-$version-$PythonAbi-$PythonAbi-$platform.whl"
    if ($files.Count -ne 1 -or -not $files.Contains($wheel)) {
        throw "Expected exactly the runtime wheel $wheel"
    }
} else {
    if ($PythonAbi) { throw "Only Python artifacts may specify a Python ABI." }
    $native = if ($Kind -eq "codegen") { "dynwinrt-codegen.exe" } else { "dynwinrt.win32-$Architecture-msvc.node" }
    if (-not $files.Contains($native)) { throw "Missing $native in $payload" }
    $nativeFiles = @($files.Keys | Where-Object { $_ -match '\.(exe|node)$' })
    if ($nativeFiles.Count -ne 1) { throw "Expected one native binary in $payload" }
    if ($Kind -eq "codegen" -and ($Features -ne "production" -or $files.Count -ne 1)) {
        throw "Codegen artifacts must contain only the production executable."
    }
    $bytes = [IO.File]::ReadAllBytes((Join-Path $payload $native))
    if ($bytes.Length -lt 64 -or [BitConverter]::ToUInt16($bytes, 0) -ne 0x5a4d) {
        throw "Invalid native PE artifact: $native"
    }
    $pe = [BitConverter]::ToInt32($bytes, 0x3c)
    $machine = if ($Architecture -eq "x64") { 0x8664 } else { 0xaa64 }
    if ($pe -lt 64 -or $pe -gt $bytes.Length - 24 -or
        [BitConverter]::ToUInt32($bytes, $pe) -ne 0x4550 -or
        [BitConverter]::ToUInt16($bytes, $pe + 4) -ne $machine) {
        throw "Invalid PE architecture for $native (expected $Architecture)"
    }
}

if ($Mode -eq "Write") {
    if (Test-Path -LiteralPath $manifestPath) { throw "Artifact manifest already exists: $manifestPath" }
    $identity.files = $files
    $identity | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath $manifestPath -Encoding utf8
} else {
    $manifest = Get-Content -LiteralPath $manifestPath -Raw | ConvertFrom-Json -AsHashtable
    foreach ($key in $identity.Keys) {
        if (-not $manifest.ContainsKey($key) -or $manifest[$key] -cne $identity[$key]) {
            throw "Artifact $key mismatch in $manifestPath"
        }
    }
    if ($manifest.files -isnot [System.Collections.IDictionary] -or
        @(Compare-Object @($files.Keys | Sort-Object) @($manifest.files.Keys | Sort-Object)).Count) {
        throw "Artifact file set mismatch in $manifestPath"
    }
    foreach ($file in $files.Keys) {
        if ($manifest.files[$file] -cne $files[$file]) { throw "Artifact SHA256 mismatch: $file" }
    }
}
Write-Host "$Mode verified $Kind $Architecture $Profile $Features ($($files.Count) files, $($identity.commit))"
