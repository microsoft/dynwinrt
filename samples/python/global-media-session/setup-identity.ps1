#!/usr/bin/env pwsh

param(
    [string]$Python = "python"
)

$ErrorActionPreference = "Stop"
if (-not [System.IO.Path]::IsPathRooted($Python)) {
    $Python = (Get-Command $Python -ErrorAction Stop).Source
}
$Python = [System.IO.Path]::GetFullPath($Python)
$externalLocation = Split-Path $Python -Parent
$executable = Split-Path $Python -Leaf

$identity = Join-Path $PSScriptRoot ".identity"
$assets = Join-Path $identity "Assets"
New-Item -ItemType Directory -Force -Path $identity, $assets | Out-Null

Add-Type -AssemblyName System.Drawing
foreach ($asset in @(
    @{ Name = "StoreLogo.png"; Size = 50 },
    @{ Name = "Square44x44Logo.png"; Size = 44 },
    @{ Name = "Square150x150Logo.png"; Size = 150 }
)) {
    $path = Join-Path $assets $asset.Name
    $bitmap = [System.Drawing.Bitmap]::new($asset.Size, $asset.Size)
    $graphics = [System.Drawing.Graphics]::FromImage($bitmap)
    try {
        $graphics.Clear([System.Drawing.Color]::FromArgb(0, 95, 184))
        $font = [System.Drawing.Font]::new(
            "Segoe UI",
            [single]($asset.Size * 0.45),
            [System.Drawing.FontStyle]::Bold,
            [System.Drawing.GraphicsUnit]::Pixel)
        $brush = [System.Drawing.SolidBrush]::new([System.Drawing.Color]::White)
        $format = [System.Drawing.StringFormat]::new()
        try {
            $format.Alignment = [System.Drawing.StringAlignment]::Center
            $format.LineAlignment = [System.Drawing.StringAlignment]::Center
            $graphics.DrawString(
                "P",
                $font,
                $brush,
                [System.Drawing.RectangleF]::new(0, 0, $asset.Size, $asset.Size),
                $format)
        } finally {
            $format.Dispose()
            $brush.Dispose()
            $font.Dispose()
        }
        $bitmap.Save($path, [System.Drawing.Imaging.ImageFormat]::Png)
    } finally {
        $graphics.Dispose()
        $bitmap.Dispose()
    }
}

$template = Get-Content (Join-Path $PSScriptRoot "appxmanifest.xml.in") -Raw
$manifest = Join-Path $identity "AppxManifest.xml"
Set-Content `
    -LiteralPath $manifest `
    -Value $template.Replace("__EXECUTABLE__", $executable) `
    -Encoding utf8

Get-AppxPackage -Name "dynwinrt-python-gsmtc-sample" -ErrorAction SilentlyContinue |
    Remove-AppxPackage -ErrorAction SilentlyContinue
Add-AppxPackage -Register $manifest -ExternalLocation $externalLocation

Write-Host "Registered the globalMediaControl identity for $Python"
