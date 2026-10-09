# Photograph the window, without a person and without the compositor.
#
# eframe draws with OpenGL, and an unoccluded GL surface reaches the display
# without passing through the compositor, so reading the screen from outside
# comes back black. The app takes the picture itself: `TGA_SHOT` makes it ask
# egui for the frame it drew, write it as raw RGBA, and quit. See
# `crates/tga-app/src/shot.rs`. This turns that into a PNG.
#
#   tools\shot.ps1 -Folder "L:\9 telegram export" -Out shots\library.png
#   tools\shot.ps1 -Size 900x620          # the floor, where layouts break
#
# Nothing is analysed: the window opens on the folder, waits for its counts,
# and closes.

[CmdletBinding()]
param(
    [string]$Folder = '',
    [string]$Out = "$env:TEMP\tga-shot.png",
    [string]$Exe = "target\debug\TelegramAnalyser.exe",
    # WxH. Empty keeps the default size.
    [string]$Size = '',
    # Counting forty databases on a USB drive takes a few seconds; the app quits
    # by itself as soon as it has written the frame, so this is only a ceiling.
    [int]$TimeoutSeconds = 60
)

$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing

$raw = [System.IO.Path]::Combine($env:TEMP, "tga-shot.raw")
Remove-Item $raw -Force -ErrorAction SilentlyContinue

$env:TGA_SHOT = $raw
if ($Folder) { $env:TGA_SHOT_PATH = $Folder }
if ($Size) { $env:TGA_SHOT_SIZE = $Size }
try {
    $p = Start-Process -FilePath $Exe -PassThru
    if (-not $p.WaitForExit($TimeoutSeconds * 1000)) {
        Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue
        throw "the window did not write a frame within ${TimeoutSeconds}s"
    }
}
finally {
    Remove-Item Env:TGA_SHOT, Env:TGA_SHOT_PATH, Env:TGA_SHOT_SIZE -ErrorAction SilentlyContinue
}

if (-not (Test-Path $raw)) { throw "no frame at $raw" }

$bytes = [System.IO.File]::ReadAllBytes($raw)
if ($bytes.Length -lt 12 -or [System.Text.Encoding]::ASCII.GetString($bytes, 0, 4) -ne 'TGXS') {
    throw "$raw is not a frame this script wrote"
}
$w = [BitConverter]::ToUInt32($bytes, 4)
$h = [BitConverter]::ToUInt32($bytes, 8)
$expected = 12 + $w * $h * 4
if ($bytes.Length -ne $expected) { throw "expected $expected bytes for ${w}x${h}, got $($bytes.Length)" }

# RGBA on the wire, BGRA in a GDI+ bitmap, so the two colour channels swap.
$pixels = New-Object byte[] ($w * $h * 4)
for ($i = 0; $i -lt $w * $h; $i++) {
    $s = 12 + $i * 4
    $d = $i * 4
    $pixels[$d] = $bytes[$s + 2]
    $pixels[$d + 1] = $bytes[$s + 1]
    $pixels[$d + 2] = $bytes[$s]
    $pixels[$d + 3] = $bytes[$s + 3]
}

$bmp = New-Object System.Drawing.Bitmap ([int]$w), ([int]$h), ([System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
$rect = New-Object System.Drawing.Rectangle 0, 0, ([int]$w), ([int]$h)
$data = $bmp.LockBits($rect, [System.Drawing.Imaging.ImageLockMode]::WriteOnly, $bmp.PixelFormat)
[System.Runtime.InteropServices.Marshal]::Copy($pixels, 0, $data.Scan0, $pixels.Length)
$bmp.UnlockBits($data)

$dir = Split-Path -Parent $Out
if ($dir -and -not (Test-Path $dir)) { New-Item -ItemType Directory -Force $dir | Out-Null }
$bmp.Save($Out, [System.Drawing.Imaging.ImageFormat]::Png)
$bmp.Dispose()
Remove-Item $raw -Force -ErrorAction SilentlyContinue

"$Out  (${w}x${h})"
