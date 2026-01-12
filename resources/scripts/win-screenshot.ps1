# win-screenshot.ps1
# Usage: powershell -ExecutionPolicy Bypass -File win-screenshot.ps1 <OutputPath>

param (
    [string]$OutputPath
)

Add-Type -AssemblyName System.Windows.Forms

# Clear clipboard
[System.Windows.Forms.Clipboard]::Clear()

# Launch Snipping Tool (Win+Shift+S behavior)
# In recent Windows 10/11 versions, we can invoke the screen clipping URI
Start-Process "ms-screenclip:"

# Wait for clipboard data (timeout 30s)
$timeout = 30
$timer = 0
$captured = $false

while ($timer -lt $timeout) {
    if ([System.Windows.Forms.Clipboard]::ContainsImage()) {
        try {
            $image = [System.Windows.Forms.Clipboard]::GetImage()
            $image.Save($OutputPath, [System.Drawing.Imaging.ImageFormat]::Png)
            $captured = $true
            break
        } catch {
            Write-Error "Failed to save image from clipboard."
            exit 1
        }
    }
    Start-Sleep -Seconds 1
    $timer++
}

if (-not $captured) {
    Write-Error "Timeout waiting for screenshot."
    exit 1
}

Write-Host "Screenshot saved to $OutputPath"
