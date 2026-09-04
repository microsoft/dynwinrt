function Resolve-DynWinRTWinAppSdkInputs {
    param(
        [Parameter(Mandatory)]
        [string]$SampleRoot,

        [Parameter(Mandatory)]
        [string[]]$PrimaryWinmdNames,

        [ValidateSet("x64", "arm64")]
        [string]$Architecture = "x64"
    )

    $winapp = Join-Path $SampleRoot ".winapp"
    $lockPath = Join-Path $winapp "winmds.lock.json"
    if (-not (Test-Path -LiteralPath $lockPath -PathType Leaf)) {
        throw "WinApp SDK metadata was not restored. Run 'winapp restore' in $SampleRoot."
    }

    try {
        $lock = Get-Content -LiteralPath $lockPath -Raw | ConvertFrom-Json
    } catch {
        throw "Could not read WinApp metadata lock file '$lockPath': $($_.Exception.Message)"
    }

    $packageNames = @(
        "Microsoft.WindowsAppSDK.Foundation",
        "Microsoft.WindowsAppSDK.InteractiveExperiences",
        "Microsoft.WindowsAppSDK.WinUI"
    )
    $winmds = @(
        $lock.packages |
            Where-Object { $_.name -in $packageNames } |
            ForEach-Object { $_.winmds } |
            Where-Object { $_ -and (Test-Path -LiteralPath $_ -PathType Leaf) } |
            Group-Object { Split-Path -Leaf $_ } |
            ForEach-Object {
                $_.Group |
                    Sort-Object -Descending |
                    Select-Object -First 1
            }
    )
    if ($winmds.Count -eq 0) {
        throw "WinApp metadata lock file '$lockPath' contains no usable Windows App SDK WinMD paths."
    }

    $primaryWinmds = @{}
    foreach ($name in $PrimaryWinmdNames) {
        $matches = @($winmds | Where-Object { (Split-Path -Leaf $_) -eq $name })
        if ($matches.Count -ne 1) {
            throw "Expected one '$name' entry in '$lockPath', found $($matches.Count)."
        }
        $primaryWinmds[$name] = $matches[0]
    }

    $primaryPaths = @($primaryWinmds.Values)
    $references = @($winmds | Where-Object { $_ -notin $primaryPaths })
    $refList = Join-Path $winapp "dynwinrt-codegen-refs.txt"
    [System.IO.File]::WriteAllLines(
        $refList,
        $references,
        [System.Text.UTF8Encoding]::new($false)
    )

    $bootstrapDll = Join-Path `
        (Join-Path (Join-Path $winapp "bin") $Architecture) `
        "Microsoft.WindowsAppRuntime.Bootstrap.dll"
    if (-not (Test-Path -LiteralPath $bootstrapDll -PathType Leaf)) {
        throw "WinApp bootstrap DLL was not found at '$bootstrapDll'. Run 'winapp restore' for architecture '$Architecture'."
    }

    [pscustomobject]@{
        PrimaryWinmds = $primaryWinmds
        RefList = $refList
        BootstrapDll = $bootstrapDll
    }
}
