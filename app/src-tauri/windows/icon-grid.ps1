param([int]$cols = 60, [int]$rows = 28, [string]$out = (Join-Path $PSScriptRoot "icon-grid.txt"))
# For scripts/readme-art.py. Samples the app icon into a grid: B = bird (dark shape), S = the blue square, space = outside.
Add-Type -AssemblyName System.Drawing
$img = New-Object System.Drawing.Bitmap (Join-Path $PSScriptRoot "..\icons\icon.png")
$lines = @()
for ($r = 0; $r -lt $rows; $r++) {
  $s = ""
  for ($c = 0; $c -lt $cols; $c++) {
    $x = [int](($c + 0.5) * $img.Width / $cols); $y = [int](($r + 0.5) * $img.Height / $rows)
    $p = $img.GetPixel($x, $y)
    if ($p.A -lt 128) { $s += " " }
    elseif ((0.3 * $p.R + 0.59 * $p.G + 0.11 * $p.B) -lt 30) { $s += "B" }
    else { $s += "S" }
  }
  $lines += $s
}
$lines | Set-Content $out -Encoding ascii
$lines
