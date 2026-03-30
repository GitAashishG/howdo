# install.ps1 — Quick installer for howdo on Windows
# Usage: irm https://raw.githubusercontent.com/GitAashishG/howdo/main/install.ps1 | iex

$ErrorActionPreference = "Stop"

$repo = "GitAashishG/howdo"
$binary = "howdo.exe"
$installDir = "$env:LOCALAPPDATA\Programs\howdo"

try {

# Detect architecture
$arch = if ([Environment]::Is64BitOperatingSystem) { "x86_64" } else { "x86" }
if ($arch -ne "x86_64") {
    Write-Host "Unsupported architecture: $arch. Only 64-bit Windows is supported." -ForegroundColor Red
    return
}
$target = "x86_64-pc-windows-msvc"

# Get latest release tag
Write-Host "Finding latest release..." -ForegroundColor Cyan
$release = Invoke-RestMethod -Uri "https://api.github.com/repos/$repo/releases/latest" -Headers @{ "User-Agent" = "howdo-installer" }
$tag = $release.tag_name
$asset = $release.assets | Where-Object { $_.name -like "*$target*" } | Select-Object -First 1

if (-not $asset) {
    Write-Host "No binary found for $target in release $tag" -ForegroundColor Red
    return
}

# Download
Write-Host "Downloading howdo $tag for $target..." -ForegroundColor Cyan
$tmpFile = Join-Path $env:TEMP $binary
Invoke-WebRequest -Uri $asset.browser_download_url -OutFile $tmpFile -UseBasicParsing

# Install
if (-not (Test-Path $installDir)) {
    New-Item -ItemType Directory -Path $installDir -Force | Out-Null
}
Move-Item -Path $tmpFile -Destination (Join-Path $installDir $binary) -Force

# Add to PATH if not already there
$userPath = [Environment]::GetEnvironmentVariable("Path", "User")
if ($userPath -notlike "*$installDir*") {
    [Environment]::SetEnvironmentVariable("Path", "$userPath;$installDir", "User")
    Write-Host "Added $installDir to your PATH." -ForegroundColor Green
    Write-Host "Restart your terminal for PATH changes to take effect." -ForegroundColor Yellow
} else {
    Write-Host "$installDir is already in your PATH." -ForegroundColor Green
}

Write-Host ""
Write-Host "Installed howdo $tag!" -ForegroundColor Green
Write-Host "Run 'howdo /config' to set up your LLM provider." -ForegroundColor Cyan

} catch {
    Write-Host "Installation failed: $_" -ForegroundColor Red
}
