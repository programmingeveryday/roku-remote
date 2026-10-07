Add-Type -AssemblyName System.Drawing

$srcPath = "C:\Users\edwar\.gemini\antigravity\brain\0b05c700-649c-48e1-a9f6-7170697701c2\github_social_card_1791344417121.jpg"
$srcBmp = New-Object System.Drawing.Bitmap($srcPath)

# Source is 1376 x 768.
# Target aspect ratio is 2:1 (1280 x 640).
# For a 768 height, a 2:1 canvas is 1536 x 768.
# We pad (1536 - 1376) / 2 = 80px on left and right, then downscale smoothly to 1280 x 640.

$canvas1536 = New-Object System.Drawing.Bitmap(1536, 768, [System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
$g1 = [System.Drawing.Graphics]::FromImage($canvas1536)
$g1.SmoothingMode = [System.Drawing.Drawing2D.SmoothingMode]::HighQuality
$g1.InterpolationMode = [System.Drawing.Drawing2D.InterpolationMode]::HighQualityBicubic
$g1.PixelOffsetMode = [System.Drawing.Drawing2D.PixelOffsetMode]::HighQuality

# Background fill with edge color
$bgColor = [System.Drawing.Color]::FromArgb(255, 7, 5, 18)
$g1.Clear($bgColor)

# Draw the left 1-pixel column repeated across the left 80px
$leftColRect = New-Object System.Drawing.Rectangle(0, 0, 1, 768)
$leftDestRect = New-Object System.Drawing.Rectangle(0, 0, 80, 768)
$g1.DrawImage($srcBmp, $leftDestRect, $leftColRect, [System.Drawing.GraphicsUnit]::Pixel)

# Draw the right 1-pixel column repeated across the right 80px
$rightColRect = New-Object System.Drawing.Rectangle(1375, 0, 1, 768)
$rightDestRect = New-Object System.Drawing.Rectangle(1456, 0, 80, 768)
$g1.DrawImage($srcBmp, $rightDestRect, $rightColRect, [System.Drawing.GraphicsUnit]::Pixel)

# Draw the source image in the center
$srcDestRect = New-Object System.Drawing.Rectangle(80, 0, 1376, 768)
$srcRect = New-Object System.Drawing.Rectangle(0, 0, 1376, 768)
$g1.DrawImage($srcBmp, $srcDestRect, $srcRect, [System.Drawing.GraphicsUnit]::Pixel)
$g1.Dispose()

# Now downscale cleanly to exactly 1280 x 640
$targetBmp = New-Object System.Drawing.Bitmap(1280, 640, [System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
$g2 = [System.Drawing.Graphics]::FromImage($targetBmp)
$g2.SmoothingMode = [System.Drawing.Drawing2D.SmoothingMode]::HighQuality
$g2.InterpolationMode = [System.Drawing.Drawing2D.InterpolationMode]::HighQualityBicubic
$g2.PixelOffsetMode = [System.Drawing.Drawing2D.PixelOffsetMode]::HighQuality

$finalDestRect = New-Object System.Drawing.Rectangle(0, 0, 1280, 640)
$canvasRect = New-Object System.Drawing.Rectangle(0, 0, 1536, 768)
$g2.DrawImage($canvas1536, $finalDestRect, $canvasRect, [System.Drawing.GraphicsUnit]::Pixel)
$g2.Dispose()

# Save PNG
$outPng = "docs\images\github-social-preview.png"
$targetBmp.Save($outPng, [System.Drawing.Imaging.ImageFormat]::Png)

# Save High Quality JPEG
$outJpg = "docs\images\github-social-preview.jpg"
$encoderParams = New-Object System.Drawing.Imaging.EncoderParameters(1)
$encoderParams.Param[0] = New-Object System.Drawing.Imaging.EncoderParameter([System.Drawing.Imaging.Encoder]::Quality, [long]95)
$jpegCodec = [System.Drawing.Imaging.ImageCodecInfo]::GetImageEncoders() | Where-Object { $_.MimeType -eq "image/jpeg" }
$targetBmp.Save($outJpg, $jpegCodec, $encoderParams)

$targetBmp.Dispose()
$canvas1536.Dispose()
$srcBmp.Dispose()

Write-Host "Generated 1280x640 GitHub social preview images:"
Write-Host "  PNG: $outPng"
Write-Host "  JPG: $outJpg"
