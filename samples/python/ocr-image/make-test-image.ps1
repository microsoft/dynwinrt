#!/usr/bin/env pwsh

param(
    [string]$Output = (Join-Path $PSScriptRoot "sample.png")
)

$ErrorActionPreference = "Stop"
Add-Type -AssemblyName System.Drawing

$bitmap = [System.Drawing.Bitmap]::new(900, 180)
$graphics = [System.Drawing.Graphics]::FromImage($bitmap)
try {
    $graphics.Clear([System.Drawing.Color]::White)
    $graphics.TextRenderingHint = [System.Drawing.Text.TextRenderingHint]::AntiAliasGridFit
    $font = [System.Drawing.Font]::new(
        "Segoe UI",
        52,
        [System.Drawing.FontStyle]::Bold,
        [System.Drawing.GraphicsUnit]::Pixel)
    $brush = [System.Drawing.SolidBrush]::new([System.Drawing.Color]::Black)
    try {
        $graphics.DrawString("DYNWINRT OCR 42", $font, $brush, 30, 45)
    } finally {
        $brush.Dispose()
        $font.Dispose()
    }
    $bitmap.Save(
        [System.IO.Path]::GetFullPath($Output),
        [System.Drawing.Imaging.ImageFormat]::Png)
} finally {
    $graphics.Dispose()
    $bitmap.Dispose()
}

Write-Host "Created OCR test image at $Output"
