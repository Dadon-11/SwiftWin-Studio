# Draws the installer's side banner (164x314) and header (150x57) from the app icon.
# Run again after changing the icon:  powershell -File app\src-tauri\windows\make-installer-art.ps1
Add-Type -AssemblyName System.Drawing
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
$icon = [System.Drawing.Image]::FromFile((Join-Path $here "..\icons\icon.png"))

function Save-Bmp($bmp, $name) {
    # NSIS wants a plain 24-bit BMP.
    $out = New-Object System.Drawing.Bitmap $bmp.Width, $bmp.Height, ([System.Drawing.Imaging.PixelFormat]::Format24bppRgb)
    $g = [System.Drawing.Graphics]::FromImage($out); $g.DrawImage($bmp, 0, 0, $bmp.Width, $bmp.Height); $g.Dispose()
    $out.Save((Join-Path $here $name), [System.Drawing.Imaging.ImageFormat]::Bmp); $out.Dispose()
}

function New-Canvas($w, $h) {
    $b = New-Object System.Drawing.Bitmap $w, $h
    $g = [System.Drawing.Graphics]::FromImage($b)
    $g.SmoothingMode = "AntiAlias"; $g.InterpolationMode = "HighQualityBicubic"; $g.TextRenderingHint = "AntiAliasGridFit"
    return $b, $g
}

# Side banner: the welcome and finish pages.
$b, $g = New-Canvas 164 314
$rect = New-Object System.Drawing.Rectangle 0, 0, 164, 314
$bg = New-Object System.Drawing.Drawing2D.LinearGradientBrush $rect, ([System.Drawing.Color]::FromArgb(10, 18, 40)), ([System.Drawing.Color]::FromArgb(22, 58, 140)), 90
$g.FillRectangle($bg, $rect)
$g.DrawImage($icon, 34, 56, 96, 96)
$center = New-Object System.Drawing.StringFormat; $center.Alignment = "Center"
$title = New-Object System.Drawing.Font "Segoe UI Semibold", 13
$sub = New-Object System.Drawing.Font "Segoe UI", 8
$g.DrawString("SwiftWin Studio", $title, [System.Drawing.Brushes]::White, (New-Object System.Drawing.RectangleF 0, 170, 164, 30), $center)
$soft = New-Object System.Drawing.SolidBrush ([System.Drawing.Color]::FromArgb(190, 205, 235))
$g.DrawString("SwiftUI iPhone apps,`nbuilt on Windows", $sub, $soft, (New-Object System.Drawing.RectangleF 0, 202, 164, 40), $center)
$g.DrawString("Open source $([char]0x00B7) MIT", $sub, $soft, (New-Object System.Drawing.RectangleF 0, 286, 164, 20), $center)
$g.Dispose(); Save-Bmp $b "installer-sidebar.bmp"; $b.Dispose()

# Header: the top-right corner of the other pages (white background).
$b, $g = New-Canvas 150 57
$g.Clear([System.Drawing.Color]::White)
$g.DrawImage($icon, 98, 6, 45, 45)
$g.Dispose(); Save-Bmp $b "installer-header.bmp"; $b.Dispose()

$icon.Dispose()
Write-Host "wrote installer-sidebar.bmp and installer-header.bmp"
