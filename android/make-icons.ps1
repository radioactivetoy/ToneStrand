# Regenerates the launcher icons (square and round PNGs per density) from assets/logos/AppIcon.jpg.
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing
$src = [System.Drawing.Image]::FromFile((Resolve-Path "$PSScriptRoot\..\assets\logos\AppIcon.jpg"))
$res = Join-Path $PSScriptRoot 'app\app\src\main\res'
$sizes = [ordered]@{ mdpi = 48; hdpi = 72; xhdpi = 96; xxhdpi = 144; xxxhdpi = 192 }
function New-Icon([int]$size, [bool]$round, [string]$path) {
    $bmp = New-Object System.Drawing.Bitmap $size, $size, ([System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $g.InterpolationMode = 'HighQualityBicubic'; $g.SmoothingMode = 'HighQuality'
    $g.PixelOffsetMode = 'HighQuality'; $g.CompositingQuality = 'HighQuality'
    $g.Clear([System.Drawing.Color]::Transparent)
    if ($round) {
        $gp = New-Object System.Drawing.Drawing2D.GraphicsPath
        $gp.AddEllipse(0, 0, $size - 1, $size - 1)
        $g.SetClip($gp)
    }
    $g.DrawImage($src, 0, 0, $size, $size)
    $g.Dispose()
    $bmp.Save($path, [System.Drawing.Imaging.ImageFormat]::Png)
    $bmp.Dispose()
}
foreach ($d in $sizes.Keys) {
    $dir = Join-Path $res "mipmap-$d"
    New-Item -ItemType Directory -Force $dir | Out-Null
    New-Icon $sizes[$d] $false (Join-Path $dir 'ic_launcher.png')
    New-Icon $sizes[$d] $true (Join-Path $dir 'ic_launcher_round.png')
}
$src.Dispose()
