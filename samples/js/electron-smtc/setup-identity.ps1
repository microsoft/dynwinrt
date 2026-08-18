# Copyright (c) Microsoft Corporation.
# Licensed under the MIT License.

$ErrorActionPreference = "Stop"
$assets = Join-Path $PSScriptRoot "Assets"
New-Item $assets -ItemType Directory -Force | Out-Null

Add-Type -AssemblyName System.Drawing

function New-SampleLogo {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)][int]$Size,
        [System.Drawing.Color]$BackgroundColor = [System.Drawing.Color]::FromArgb(0, 95, 184),
        [string]$Label = ""
    )

    $bitmap = [System.Drawing.Bitmap]::new($Size, $Size)
    $graphics = [System.Drawing.Graphics]::FromImage($bitmap)
    try {
        $graphics.SmoothingMode = [System.Drawing.Drawing2D.SmoothingMode]::AntiAlias
        $graphics.Clear($BackgroundColor)
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
        if ($Label) {
            $font = [System.Drawing.Font]::new(
                "Segoe UI",
                [single]($Size * 0.12),
                [System.Drawing.FontStyle]::Bold,
                [System.Drawing.GraphicsUnit]::Pixel)
            $brush = [System.Drawing.SolidBrush]::new([System.Drawing.Color]::White)
            $format = [System.Drawing.StringFormat]::new()
            try {
                $format.Alignment = [System.Drawing.StringAlignment]::Center
                $format.LineAlignment = [System.Drawing.StringAlignment]::Center
                $bounds = [System.Drawing.RectangleF]::new(
                    0,
                    [single]($Size * 0.72),
                    $Size,
                    [single]($Size * 0.18))
                $graphics.DrawString($Label, $font, $brush, $bounds, $format)
            }
            finally {
                $format.Dispose()
                $brush.Dispose()
                $font.Dispose()
            }
        }
        $bitmap.Save($Path, [System.Drawing.Imaging.ImageFormat]::Png)
    }
    finally {
        $graphics.Dispose()
        $bitmap.Dispose()
    }
}

function New-AlbumArtwork {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)][System.Drawing.Color]$StartColor,
        [Parameter(Mandatory = $true)][System.Drawing.Color]$EndColor,
        [Parameter(Mandatory = $true)][System.Drawing.Color]$AccentColor,
        [Parameter(Mandatory = $true)][string]$TrackNumber
    )

    $size = 512
    $bitmap = [System.Drawing.Bitmap]::new($size, $size)
    $graphics = [System.Drawing.Graphics]::FromImage($bitmap)
    try {
        $graphics.SmoothingMode = [System.Drawing.Drawing2D.SmoothingMode]::AntiAlias
        $bounds = [System.Drawing.Rectangle]::new(0, 0, $size, $size)
        $gradient = [System.Drawing.Drawing2D.LinearGradientBrush]::new(
            $bounds,
            $StartColor,
            $EndColor,
            [System.Drawing.Drawing2D.LinearGradientMode]::ForwardDiagonal)
        try {
            $graphics.FillRectangle($gradient, $bounds)
        }
        finally {
            $gradient.Dispose()
        }

        $glow = [System.Drawing.SolidBrush]::new(
            [System.Drawing.Color]::FromArgb(36, 255, 255, 255))
        try {
            $graphics.FillEllipse($glow, -110, 35, 390, 390)
            $graphics.FillEllipse($glow, 270, 255, 330, 330)
        }
        finally {
            $glow.Dispose()
        }

        $arcPen = [System.Drawing.Pen]::new(
            [System.Drawing.Color]::FromArgb(
                205,
                $AccentColor.R,
                $AccentColor.G,
                $AccentColor.B),
            14)
        try {
            $arcPen.StartCap = [System.Drawing.Drawing2D.LineCap]::Round
            $arcPen.EndCap = [System.Drawing.Drawing2D.LineCap]::Round
            for ($index = 0; $index -lt 5; $index++) {
                $inset = 58 + $index * 42
                $diameter = $size - 2 * $inset
                $graphics.DrawArc(
                    $arcPen,
                    $inset,
                    $inset,
                    $diameter,
                    $diameter,
                    205 + $index * 11,
                    235)
            }
        }
        finally {
            $arcPen.Dispose()
        }

        $linePen = [System.Drawing.Pen]::new(
            [System.Drawing.Color]::FromArgb(110, 255, 255, 255),
            5)
        try {
            for ($index = 0; $index -lt 7; $index++) {
                $x = 58 + $index * 32
                $height = 26 + (($index * 29) % 72)
                $graphics.DrawLine($linePen, $x, 396 - $height, $x, 396)
            }
        }
        finally {
            $linePen.Dispose()
        }

        $numberFont = [System.Drawing.Font]::new(
            "Segoe UI Variable Display",
            68,
            [System.Drawing.FontStyle]::Bold,
            [System.Drawing.GraphicsUnit]::Pixel)
        $captionFont = [System.Drawing.Font]::new(
            "Segoe UI",
            22,
            [System.Drawing.FontStyle]::Bold,
            [System.Drawing.GraphicsUnit]::Pixel)
        $textBrush = [System.Drawing.SolidBrush]::new(
            [System.Drawing.Color]::FromArgb(225, 255, 255, 255))
        try {
            $graphics.DrawString(
                $TrackNumber,
                $numberFont,
                $textBrush,
                [single]350,
                [single]46)
            $graphics.DrawString(
                "RUNTIME RADIO",
                $captionFont,
                $textBrush,
                [single]56,
                [single]432)
        }
        finally {
            $textBrush.Dispose()
            $captionFont.Dispose()
            $numberFont.Dispose()
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
New-AlbumArtwork `
    (Join-Path $assets "AlbumBlue.png") `
    ([System.Drawing.Color]::FromArgb(0, 67, 135)) `
    ([System.Drawing.Color]::FromArgb(0, 153, 188)) `
    ([System.Drawing.Color]::FromArgb(126, 231, 255)) `
    "01"
New-AlbumArtwork `
    (Join-Path $assets "AlbumPurple.png") `
    ([System.Drawing.Color]::FromArgb(73, 34, 128)) `
    ([System.Drawing.Color]::FromArgb(182, 64, 151)) `
    ([System.Drawing.Color]::FromArgb(255, 171, 228)) `
    "02"
New-AlbumArtwork `
    (Join-Path $assets "AlbumOrange.png") `
    ([System.Drawing.Color]::FromArgb(120, 43, 14)) `
    ([System.Drawing.Color]::FromArgb(229, 119, 36)) `
    ([System.Drawing.Color]::FromArgb(255, 224, 151)) `
    "03"

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
