Add-Type -AssemblyName System.Drawing

$srcPath = "C:\Users\edwar\.gemini\antigravity\brain\0b05c700-649c-48e1-a9f6-7170697701c2\roku_remote_icon_1791342264702.jpg"
$srcBmp = New-Object System.Drawing.Bitmap($srcPath)

function Create-RoundedIcon([int]$size, [int]$cornerRadius) {
    $bmp = New-Object System.Drawing.Bitmap($size, $size, [System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $g.SmoothingMode = [System.Drawing.Drawing2D.SmoothingMode]::AntiAlias
    $g.InterpolationMode = [System.Drawing.Drawing2D.InterpolationMode]::HighQualityBicubic
    $g.PixelOffsetMode = [System.Drawing.Drawing2D.PixelOffsetMode]::HighQuality
    $g.Clear([System.Drawing.Color]::Transparent)

    $path = New-Object System.Drawing.Drawing2D.GraphicsPath
    $d = $cornerRadius * 2
    $path.AddArc(0, 0, $d, $d, 180, 90)
    $path.AddArc($size - $d, 0, $d, $d, 270, 90)
    $path.AddArc($size - $d, $size - $d, $d, $d, 0, 90)
    $path.AddArc(0, $size - $d, $d, $d, 90, 90)
    $path.CloseFigure()

    $g.SetClip($path)
    $cropRect = New-Object System.Drawing.Rectangle(150, 150, 724, 724)
    $destRect = New-Object System.Drawing.Rectangle(0, 0, $size, $size)
    $g.DrawImage($srcBmp, $destRect, $cropRect, [System.Drawing.GraphicsUnit]::Pixel)
    $g.Dispose()
    return $bmp
}

$bmp256 = Create-RoundedIcon 256 56
$bmp256.Save("assets\icon.png", [System.Drawing.Imaging.ImageFormat]::Png)

$bmp128 = Create-RoundedIcon 128 28
$bmp128.Save("assets\icon-128.png", [System.Drawing.Imaging.ImageFormat]::Png)

$bmp48 = Create-RoundedIcon 48 10
$bmp32 = Create-RoundedIcon 32 7
$bmp16 = Create-RoundedIcon 16 4

# Create multi-size ICO file
# ICO header: 2 bytes reserved (0), 2 bytes type (1 = icon), 2 bytes image count
# Directory entry: 16 bytes per image
$sizes = @($bmp256, $bmp128, $bmp48, $bmp32, $bmp16)
$pngStreams = @()
foreach ($b in $sizes) {
    $ms = New-Object System.IO.MemoryStream
    $b.Save($ms, [System.Drawing.Imaging.ImageFormat]::Png)
    $pngStreams += $ms
}

$fs = New-Object System.IO.FileStream("assets\icon.ico", [System.IO.FileMode]::Create)
$bw = New-Object System.IO.BinaryWriter($fs)

# Header
$bw.Write([uint16]0) # Reserved
$bw.Write([uint16]1) # Type (1=ICO)
$bw.Write([uint16]$sizes.Count) # Image count

$offset = 6 + ($sizes.Count * 16)
for ($i = 0; $i -lt $sizes.Count; $i++) {
    $w = if ($sizes[$i].Width -ge 256) { [byte]0 } else { [byte]$sizes[$i].Width }
    $h = if ($sizes[$i].Height -ge 256) { [byte]0 } else { [byte]$sizes[$i].Height }
    $bw.Write($w)
    $bw.Write($h)
    $bw.Write([byte]0) # Color palette count
    $bw.Write([byte]0) # Reserved
    $bw.Write([uint16]1) # Color planes
    $bw.Write([uint16]32) # Bits per pixel
    $bw.Write([uint32]$pngStreams[$i].Length) # Image data size
    $bw.Write([uint32]$offset) # Offset
    $offset += $pngStreams[$i].Length
}

# Image data chunks
for ($i = 0; $i -lt $sizes.Count; $i++) {
    $bytes = $pngStreams[$i].ToArray()
    $bw.Write($bytes, 0, $bytes.Length)
    $pngStreams[$i].Dispose()
    $sizes[$i].Dispose()
}

$bw.Close()
$fs.Close()
$srcBmp.Dispose()

Write-Host "Generated assets\icon.png, assets\icon-128.png, and multi-resolution assets\icon.ico!"
