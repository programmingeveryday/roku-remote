# PowerShell script to compile and install Roku Remote on Windows
# Run via: powershell -ExecutionPolicy Bypass -File .\scripts\install-windows.ps1

$ErrorActionPreference = "Stop"

Write-Host "=== Roku Remote (Rust) - Windows Build & Install ===" -ForegroundColor Cyan

# 1. Check for Cargo / Rust
if (-not (Get-Command "cargo" -ErrorAction SilentlyContinue)) {
    Write-Host "Error: Cargo is not installed or not found in PATH." -ForegroundColor Red
    Write-Host "Please install Rust using https://rustup.rs" -ForegroundColor Yellow
    exit 1
}

$ScriptDir = Split-Path -Parent $MyInvocation.MyCommand.Definition
$ProjectDir = Split-Path -Parent $ScriptDir
Set-Location $ProjectDir

# 2. Build release binary
Write-Host "Building optimized release binary..." -ForegroundColor Yellow
cargo build --release

$BinSource = Join-Path $ProjectDir "target\release\roku-remote-rs.exe"
if (-not (Test-Path $BinSource)) {
    Write-Host "Error: Build output $BinSource was not found." -ForegroundColor Red
    exit 1
}

# 3. Target Install Directory (Local AppData Programs)
$InstallDir = Join-Path $env:LOCALAPPDATA "Programs\RokuRemote"
if (-not (Test-Path $InstallDir)) {
    New-Item -ItemType Directory -Path $InstallDir -Force | Out-Null
}

$BinTarget = Join-Path $InstallDir "roku-remote-rs.exe"
Write-Host "Installing executable to $BinTarget..." -ForegroundColor Yellow
Copy-Item -Path $BinSource -Destination $BinTarget -Force

# 4. Add to User PATH if not already present
$UserPath = [Environment]::GetEnvironmentVariable("Path", [EnvironmentVariableTarget]::User)
if ($UserPath -notlike "*$InstallDir*") {
    Write-Host "Adding $InstallDir to user PATH..." -ForegroundColor Yellow
    [Environment]::SetEnvironmentVariable("Path", "$UserPath;$InstallDir", [EnvironmentVariableTarget]::User)
}

# 5. Create Start Menu Shortcut
$StartMenuDir = Join-Path $env:APPDATA "Microsoft\Windows\Start Menu\Programs"
$ShortcutPath = Join-Path $StartMenuDir "Roku Remote.lnk"

Write-Host "Creating Start Menu shortcut at $ShortcutPath..." -ForegroundColor Yellow
$WshShell = New-Object -ComObject WScript.Shell
$Shortcut = $WshShell.CreateShortcut($ShortcutPath)
$Shortcut.TargetPath = $BinTarget
$Shortcut.WorkingDirectory = $InstallDir
$Shortcut.Description = "Native Roku Remote Control"
$Shortcut.Save()

Write-Host "✓ Successfully built and installed Roku Remote for Windows!" -ForegroundColor Green
Write-Host "Executable: $BinTarget"
Write-Host "Start Menu: $ShortcutPath"
Write-Host "You can now launch 'Roku Remote' from the Start Menu or run 'roku-remote-rs' in PowerShell/Command Prompt."
