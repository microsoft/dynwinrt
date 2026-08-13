# Copyright (c) Microsoft Corporation.
# Licensed under the MIT License.

$ErrorActionPreference = "Stop"
$assets = Join-Path $PSScriptRoot "Assets"
New-Item $assets -ItemType Directory -Force | Out-Null

Add-Type -AssemblyName System.Drawing

function New-SampleLogo {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)][int]$Size
    )

    $bitmap = [System.Drawing.Bitmap]::new($Size, $Size)
    $graphics = [System.Drawing.Graphics]::FromImage($bitmap)
    try {
        $graphics.Clear([System.Drawing.Color]::FromArgb(0, 95, 184))
        $pen = [System.Drawing.Pen]::new([System.Drawing.Color]::White, [Math]::Max(2, $Size / 18))
        try {
            $margin = [Math]::Max(4, [int]($Size * 0.2))
            $graphics.DrawEllipse($pen, $margin, $margin, $Size - 2 * $margin, $Size - 2 * $margin)
            $graphics.DrawLine(
                $pen,
                [int]($Size * 0.45),
                [int]($Size * 0.35),
                [int]($Size * 0.45),
                [int]($Size * 0.65))
            $graphics.DrawLine(
                $pen,
                [int]($Size * 0.45),
                [int]($Size * 0.35),
                [int]($Size * 0.68),
                [int]($Size * 0.50))
            $graphics.DrawLine(
                $pen,
                [int]($Size * 0.68),
                [int]($Size * 0.50),
                [int]($Size * 0.45),
                [int]($Size * 0.65))
        }
        finally {
            $pen.Dispose()
        }
        $bitmap.Save($Path, [System.Drawing.Imaging.ImageFormat]::Png)
    }
    finally {
        $graphics.Dispose()
        $bitmap.Dispose()
    }
}

New-SampleLogo (Join-Path $assets "StoreLogo.png") 50
New-SampleLogo (Join-Path $assets "Square44x44Logo.png") 44
New-SampleLogo (Join-Path $assets "Square150x150Logo.png") 150

Push-Location $PSScriptRoot
try {
    & npx winapp node add-electron-debug-identity --manifest appxmanifest.xml
    if ($LASTEXITCODE -ne 0) {
        throw "Creating the Electron debug identity failed with exit code $LASTEXITCODE."
    }
}
finally {
    Pop-Location
}
