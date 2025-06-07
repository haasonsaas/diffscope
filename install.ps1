# DiffScope Installation Script for Windows
# Run with: iwr -useb https://raw.githubusercontent.com/haasonsaas/diffscope/main/install.ps1 | iex

$ErrorActionPreference = "Stop"

$REPO = "haasonsaas/diffscope"
$INSTALL_DIR = "$env:LOCALAPPDATA\diffscope\bin"
$BINARY_NAME = "diffscope.exe"

Write-Host "DiffScope Installer for Windows" -ForegroundColor Cyan

# Create installation directory
if (!(Test-Path $INSTALL_DIR)) {
    Write-Host "Creating installation directory: $INSTALL_DIR"
    New-Item -ItemType Directory -Path $INSTALL_DIR -Force | Out-Null
}

# Get latest release
Write-Host "Fetching latest release..." -ForegroundColor Yellow
try {
    $releases = Invoke-RestMethod -Uri "https://api.github.com/repos/$REPO/releases/latest"
    $latestRelease = $releases.tag_name
    Write-Host "Latest release: $latestRelease" -ForegroundColor Green
} catch {
    Write-Host "Failed to fetch latest release: $_" -ForegroundColor Red
    exit 1
}

# Download URL for Windows x64
$downloadUrl = "https://github.com/$REPO/releases/download/$latestRelease/diffscope-x86_64-pc-windows-msvc.exe"

# Download binary
Write-Host "Downloading DiffScope..." -ForegroundColor Yellow
$tempFile = "$env:TEMP\diffscope.exe"
try {
    Invoke-WebRequest -Uri $downloadUrl -OutFile $tempFile -UseBasicParsing
    Write-Host "Download complete" -ForegroundColor Green
} catch {
    Write-Host "Failed to download: $_" -ForegroundColor Red
    exit 1
}

# Move to installation directory
Move-Item -Path $tempFile -Destination "$INSTALL_DIR\$BINARY_NAME" -Force

# Add to PATH if not already there
$userPath = [Environment]::GetEnvironmentVariable("Path", "User")
if ($userPath -notlike "*$INSTALL_DIR*") {
    Write-Host "Adding DiffScope to PATH..." -ForegroundColor Yellow
    [Environment]::SetEnvironmentVariable(
        "Path",
        "$userPath;$INSTALL_DIR",
        "User"
    )
    $env:Path = "$env:Path;$INSTALL_DIR"
    Write-Host "Added to PATH. You may need to restart your terminal." -ForegroundColor Yellow
}

# Verify installation
try {
    $version = & "$INSTALL_DIR\$BINARY_NAME" --version 2>&1
    Write-Host "`n✅ DiffScope installed successfully!" -ForegroundColor Green
    Write-Host "Version: $version" -ForegroundColor Cyan
    Write-Host "`nRun 'diffscope --help' to get started" -ForegroundColor Yellow
} catch {
    Write-Host "`n⚠️  Installation completed but unable to verify" -ForegroundColor Yellow
    Write-Host "You may need to restart your terminal and try again" -ForegroundColor Yellow
}