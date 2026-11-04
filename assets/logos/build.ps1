# Builds logo.png, logo-mark.png (the white shape the app tints) and icon.ico from one of the SVGs here,
# using Microsoft Edge to draw it. Usage: pwsh assets/logos/build.ps1 folder-d   (or compass-folder)
param([Parameter(Mandatory)][string]$Name)
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing

$here = $PSScriptRoot
$assets = Split-Path $here
$svg = Get-Content -Raw (Join-Path $here "$Name.svg")
$edge = @("${env:ProgramFiles(x86)}\Microsoft\Edge\Application\msedge.exe", "$env:ProgramFiles\Microsoft\Edge\Application\msedge.exe") | Where-Object { Test-Path $_ } | Select-Object -First 1
$work = Join-Path ([IO.Path]::GetTempPath()) ([Guid]::NewGuid())
New-Item -ItemType Directory $work | Out-Null

function Render($markup, $out) {
    $page = Join-Path $work 'page.html'
    Set-Content $page "<!doctype html><html><body style='margin:0;background:transparent'><div style='width:1024px;height:1024px'>$markup</div></body></html>"
    $uri = 'file:///' + ($page -replace '\\', '/')
    Start-Process $edge -Wait -ArgumentList '--headless=new', '--disable-gpu', '--hide-scrollbars', '--default-background-color=00000000', "--user-data-dir=$work\edge", '--window-size=1024,1024', "--screenshot=$out", $uri
    if (-not (Test-Path $out)) { throw "Edge made no image" }
}

function Resized($source, $size) {
    $bmp = [Drawing.Bitmap]::new($size, $size, [Drawing.Imaging.PixelFormat]::Format32bppArgb)
    $g = [Drawing.Graphics]::FromImage($bmp)
    $g.InterpolationMode = 'HighQualityBicubic'
    $g.PixelOffsetMode = 'HighQuality'
    $g.CompositingMode = 'SourceCopy'
    $g.DrawImage($source, 0, 0, $size, $size)
    $g.Dispose()
    $bmp
}

$fullPng = Join-Path $work 'full.png'
$markPng = Join-Path $work 'mark.png'
Render $svg $fullPng
Render ($svg -replace '(?m)^.*id="background".*$', '') $markPng

$full = [Drawing.Image]::FromFile($fullPng)
$mark = [Drawing.Image]::FromFile($markPng)
$b = Resized $full 256; $b.Save((Join-Path $assets 'logo.png'), [Drawing.Imaging.ImageFormat]::Png); $b.Dispose()
$b = Resized $mark 256; $b.Save((Join-Path $assets 'logo-mark.png'), [Drawing.Imaging.ImageFormat]::Png); $b.Dispose()

# An .ico is a small table followed by the images, which can each be a whole PNG.
$sizes = 16, 24, 32, 48, 64, 128, 256
$images = foreach ($s in $sizes) {
    $b = Resized $full $s
    $ms = [IO.MemoryStream]::new()
    $b.Save($ms, [Drawing.Imaging.ImageFormat]::Png)
    $b.Dispose()
    , $ms.ToArray()
}
$out = [IO.MemoryStream]::new()
$w = [IO.BinaryWriter]::new($out)
$w.Write([uint16]0); $w.Write([uint16]1); $w.Write([uint16]$sizes.Count)
$offset = 6 + 16 * $sizes.Count
for ($i = 0; $i -lt $sizes.Count; $i++) {
    $s = $sizes[$i]
    $w.Write([byte]($s % 256)); $w.Write([byte]($s % 256)); $w.Write([byte]0); $w.Write([byte]0)
    $w.Write([uint16]1); $w.Write([uint16]32)
    $w.Write([uint32]$images[$i].Length); $w.Write([uint32]$offset)
    $offset += $images[$i].Length
}
foreach ($img in $images) { $w.Write($img) }
$w.Flush()
[IO.File]::WriteAllBytes((Join-Path $assets 'icon.ico'), $out.ToArray())

$full.Dispose(); $mark.Dispose()
Remove-Item -Recurse -Force $work -ErrorAction SilentlyContinue
"Built the icons from $Name.svg"
