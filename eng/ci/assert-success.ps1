# Copyright (c) Microsoft Corporation.
# Licensed under the MIT License.

param(
    [Parameter(Mandatory)][string]$NeedsJson,
    [Parameter(Mandatory)][string[]]$RequiredJobs,
    [ValidateSet("true", "false")][string]$DocsOnly = "false",
    [string[]]$SkippableJobs = @()
)

$ErrorActionPreference = "Stop"
$needs = $NeedsJson | ConvertFrom-Json -AsHashtable
if ($needs -isnot [System.Collections.IDictionary] -or $RequiredJobs.Count -eq 0) {
    throw "A success gate requires named job results."
}
if (@(Compare-Object @($needs.Keys | Sort-Object) @($RequiredJobs | Sort-Object)).Count) {
    throw "Success gate dependencies do not match the required jobs."
}
if ($needs.changes.result -ne "success" -or $needs.changes.outputs.docs_only -cne $DocsOnly) {
    throw "A success gate requires successful, matching change classification."
}
if (@($SkippableJobs | Where-Object { $_ -notin $RequiredJobs -or $_ -eq "changes" }).Count) {
    throw "Only declared heavy dependencies may be skipped."
}
foreach ($job in $RequiredJobs) {
    $expected = if ($DocsOnly -eq "true" -and $job -in $SkippableJobs) { "skipped" } else { "success" }
    if ($needs[$job].result -ne $expected) {
        throw "Required job '$job' has result '$($needs[$job].result)'; expected '$expected'."
    }
}
Write-Host "All required jobs have the expected results (docs-only: $DocsOnly): $($RequiredJobs -join ', ')"
