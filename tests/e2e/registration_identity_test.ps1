#!/usr/bin/env pwsh
# Copyright (c) Microsoft Corporation.
# Licensed under the MIT License.

param(
    [Parameter(Mandatory)]
    [string]$WinuiWinmd,
    [Parameter(Mandatory)]
    [string]$RefList,
    [string]$WindowsWinmd = "C:\Program Files (x86)\Windows Kits\10\UnionMetadata\10.0.26100.0\Windows.winmd",
    [string]$Python = "python",
    [string]$Codegen,
    [string]$JsRuntime,
    [ValidateSet("py", "js")]
    [string[]]$Lang = @("py", "js")
)

$ErrorActionPreference = "Stop"
$root = Split-Path (Split-Path $PSScriptRoot -Parent) -Parent
$out = Join-Path $PSScriptRoot "e2e_generated\registration_identity"
if (-not $Codegen) { $Codegen = Join-Path $root "target\release\dynwinrt-codegen.exe" }
if (-not $JsRuntime) { $JsRuntime = Join-Path $root "bindings\js\dist\winrt.js" }
$Codegen = (Resolve-Path -LiteralPath $Codegen).Path
$WinuiWinmd = (Resolve-Path -LiteralPath $WinuiWinmd).Path
$WindowsWinmd = (Resolve-Path -LiteralPath $WindowsWinmd).Path
$RefList = (Resolve-Path -LiteralPath $RefList).Path
if ("js" -in $Lang) { $JsRuntime = (Resolve-Path -LiteralPath $JsRuntime).Path }
New-Item -ItemType Directory -Path $out -Force | Out-Null

function Invoke-Case([string]$Executable, [string[]]$Arguments) {
    $start = [System.Diagnostics.ProcessStartInfo]::new((Get-Command $Executable).Source)
    $start.UseShellExecute = $false
    $start.RedirectStandardOutput = $true
    $start.RedirectStandardError = $true
    $start.StandardOutputEncoding = [System.Text.Encoding]::UTF8
    $start.StandardErrorEncoding = [System.Text.Encoding]::UTF8
    $start.Environment["PYTHONIOENCODING"] = "utf-8"
    foreach ($argument in $Arguments) { $start.ArgumentList.Add($argument) }
    $process = [System.Diagnostics.Process]::Start($start)
    try {
        $stdout = $process.StandardOutput.ReadToEndAsync()
        $stderr = $process.StandardError.ReadToEndAsync()
        $timedOut = -not $process.WaitForExit(30000)
        if ($timedOut) {
            Stop-Process -Id $process.Id -Force
            $process.WaitForExit()
        }
        return @{
            exitCode = $process.ExitCode
            timedOut = $timedOut
            stdout = $stdout.GetAwaiter().GetResult()
            stderr = $stderr.GetAwaiter().GetResult()
        }
    } finally {
        $process.Dispose()
    }
}

$results = @()
foreach ($language in $Lang) {
    foreach ($name in @("windows", "winui")) {
        $directory = Join-Path $out "${name}_$language"
        $namespace = if ($name -eq "windows") { "Windows.UI.Xaml.Data" } else { "Microsoft.UI.Xaml.Data" }
        $winmd = if ($name -eq "windows") { $WindowsWinmd } else { $WinuiWinmd }
        $extra = @()
        if ($language -eq "js") {
            $extra = @("--import-name", [IO.Path]::GetRelativePath($directory, $JsRuntime).Replace("\", "/"))
        }
        & $Codegen generate --winmd $winmd --ref-list $RefList --namespace $namespace `
            --class-name INotifyPropertyChanged --lang $language --output $directory @extra
        if ($LASTEXITCODE -ne 0) { throw "Generation failed for $namespace ($language)" }
    }
    foreach ($order in @("windows", "winui", "windows,winui", "winui,windows")) {
        $result = if ($language -eq "js") {
            Invoke-Case "node" @((Join-Path $PSScriptRoot "runners\registration_identity_js.cjs"), $out, $JsRuntime, $order)
        } else {
            Invoke-Case $Python @((Join-Path $PSScriptRoot "runners\registration_identity_py.py"), $out, $order)
        }
        $result.language = $language
        $result.order = $order
        $result.pass = -not $result.timedOut -and $result.exitCode -eq 0
        $results += $result
        Write-Host "$language $order : $(if ($result.pass) { 'PASS' } else { 'FAIL' })"
        if (-not $result.pass) { Write-Host $result.stderr }
    }
}
$results | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath (Join-Path $out "results.json") -Encoding utf8
if ($results.Where({ -not $_.pass }).Count) { exit 1 }
exit 0
