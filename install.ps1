# Windows installer. Downloads an exact release asset and verifies SHA-256 before replacing anything.
# Usage: irm https://raw.githubusercontent.com/GitAashishG/howdo/main/install.ps1 | iex

$previousErrorAction = $ErrorActionPreference
$ErrorActionPreference = 'Stop'
$tempDir = $null
$staged = $null

try {
    $repo = 'GitAashishG/howdo'
    $architecture = if ($env:PROCESSOR_ARCHITEW6432) { $env:PROCESSOR_ARCHITEW6432 } else { $env:PROCESSOR_ARCHITECTURE }
    if ($architecture -ne 'AMD64') {
        throw "No prebuilt binary for $architecture. Only x86_64 Windows is supported."
    }
    $installDir = if ($env:HOWDO_INSTALL_DIR) { $env:HOWDO_INSTALL_DIR } else { Join-Path $env:LOCALAPPDATA 'Programs\howdo' }
    if (-not [System.IO.Path]::IsPathRooted($installDir)) {
        throw 'HOWDO_INSTALL_DIR must be an absolute path.'
    }
    $name = 'howdo-x86_64-pc-windows-msvc.exe'
    $headers = @{ 'User-Agent' = 'howdo-installer' }
    $release = Invoke-RestMethod -Uri "https://api.github.com/repos/$repo/releases/latest" -Headers $headers -TimeoutSec 30
    $tag = $release.tag_name
    if ($tag -notmatch '^v[0-9]+\.[0-9]+\.[0-9]+$') {
        throw 'Could not determine a stable release version.'
    }
    foreach ($assetName in @($name, 'SHA256SUMS')) {
        $assets = @($release.assets | Where-Object { $_.name -ceq $assetName })
        if ($assets.Count -ne 1 -or $assets[0].browser_download_url -cne "https://github.com/$repo/releases/download/$tag/$assetName") {
            throw "Missing, duplicate, or unexpected release asset: $assetName. Nothing was installed."
        }
    }
    $tempDir = Join-Path ([System.IO.Path]::GetTempPath()) ('howdo-install-' + [guid]::NewGuid().ToString('N'))
    New-Item -ItemType Directory -Path $tempDir | Out-Null
    $binaryFile = Join-Path $tempDir 'howdo.exe'
    $manifestFile = Join-Path $tempDir 'SHA256SUMS'
    Write-Host "Downloading howdo $tag..."
    $base = "https://github.com/$repo/releases/download/$tag"
    Invoke-WebRequest -Uri "$base/$name" -OutFile $binaryFile -Headers $headers -UseBasicParsing -TimeoutSec 120
    Invoke-WebRequest -Uri "$base/SHA256SUMS" -OutFile $manifestFile -Headers $headers -UseBasicParsing -TimeoutSec 30
    $checksums = @()
    foreach ($line in (Get-Content -LiteralPath $manifestFile)) {
        if ([string]::IsNullOrWhiteSpace($line)) { continue }
        if ($line -notmatch '^([0-9a-fA-F]{64})[ \t]+\*?([^ \t]+)$') {
            throw 'Malformed checksum manifest. Nothing was installed.'
        }
        if ($Matches[2] -ceq $name) { $checksums += $Matches[1] }
    }
    if ($checksums.Count -ne 1 -or (Get-Item -LiteralPath $binaryFile).Length -eq 0 -or
        (Get-FileHash -LiteralPath $binaryFile -Algorithm SHA256).Hash -ine $checksums[0]) {
        throw 'Checksum mismatch, missing checksum, or empty binary. Nothing was installed.'
    }
    New-Item -ItemType Directory -Path $installDir -Force | Out-Null
    $staged = Join-Path $installDir ('.howdo-' + [guid]::NewGuid().ToString('N') + '.tmp')
    Copy-Item -LiteralPath $binaryFile -Destination $staged
    $destination = Join-Path $installDir 'howdo.exe'
    if (Test-Path -LiteralPath $destination) {
        # PowerShell converts $null to an empty string for .NET string parameters.
        # NullString preserves a real null backup path, including on Windows PowerShell 5.1.
        [System.IO.File]::Replace($staged, $destination, [System.Management.Automation.Language.NullString]::Value)
    } else {
        [System.IO.File]::Move($staged, $destination)
    }
    $staged = $null
    if ($env:HOWDO_NO_PATH_UPDATE -ne '1') {
        $userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
        if (($userPath -split ';') -notcontains $installDir) {
            $newPath = if ([string]::IsNullOrEmpty($userPath)) { $installDir } else { "$userPath;$installDir" }
            [Environment]::SetEnvironmentVariable('Path', $newPath, 'User')
        }
        if (($env:Path -split ';') -notcontains $installDir) { $env:Path = "$env:Path;$installDir" }
    }
    Write-Host "Installed $tag to $destination (SHA-256 verified)."
    Write-Host "Run 'howdo /config' to configure a provider."
} finally {
    if ($staged -and (Test-Path -LiteralPath $staged)) { Remove-Item -LiteralPath $staged -Force -ErrorAction SilentlyContinue }
    if ($tempDir -and (Test-Path -LiteralPath $tempDir)) { Remove-Item -LiteralPath $tempDir -Recurse -Force -ErrorAction SilentlyContinue }
    $ErrorActionPreference = $previousErrorAction
}
