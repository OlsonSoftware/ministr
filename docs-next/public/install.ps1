# ministr installer (Windows, PowerShell) — downloads the latest CLI release.
# Usage: iwr -useb https://ministr.app/install.ps1 | iex
#
# Fetches assets from https://dl.ministr.app, a Cloudflare Worker that
# fronts the private GitHub repo's releases. The Worker auth is opaque
# to this script — all downloads are unauthenticated HTTPS GETs.
#
# Env overrides:
#   $env:MINISTR_DL_HOST  — proxy host (default https://dl.ministr.app)
#   $env:INSTALL_DIR      — install location (default $env:USERPROFILE\.ministr\bin)

$ErrorActionPreference = 'Stop'

$DlHost = if ($env:MINISTR_DL_HOST) { $env:MINISTR_DL_HOST } else { 'https://dl.ministr.app' }
$InstallDir = if ($env:INSTALL_DIR) { $env:INSTALL_DIR } else { Join-Path $env:USERPROFILE '.ministr\bin' }

function Write-Info($msg) { Write-Host $msg -ForegroundColor Blue }
function Write-Err($msg)  { Write-Host "error: $msg" -ForegroundColor Red; exit 1 }

# Architecture detection — we only ship x86_64 Windows. ARM64 Windows users
# build from source until we add that target.
switch ($env:PROCESSOR_ARCHITECTURE) {
    'AMD64' { $arch = 'x86_64' }
    'ARM64' { Write-Err 'Windows ARM64 binaries are not yet published. Build from source: cargo install --git https://github.com/OlsonSoftware/ministr ministr-cli' }
    default { Write-Err "unsupported architecture: $env:PROCESSOR_ARCHITECTURE" }
}

$archive = "ministr-$arch-pc-windows-msvc.zip"

Write-Info 'Finding latest ministr release...'
try {
    $latest = Invoke-RestMethod -Uri "$DlHost/latest" -UseBasicParsing
} catch {
    Write-Err "could not reach $DlHost/latest ($_)"
}
$tag = $latest.tag
if (-not $tag) { Write-Err "could not determine latest release tag from $DlHost/latest" }
Write-Info "Latest release: $tag"

$url = "$DlHost/$tag/$archive"
$tmpDir = Join-Path ([System.IO.Path]::GetTempPath()) ([System.Guid]::NewGuid().Guid)
New-Item -ItemType Directory -Path $tmpDir -Force | Out-Null
$tmpZip = Join-Path $tmpDir $archive

try {
    Write-Info "Downloading $archive..."
    Invoke-WebRequest -Uri $url -OutFile $tmpZip -UseBasicParsing

    Expand-Archive -Path $tmpZip -DestinationPath $tmpDir -Force

    New-Item -ItemType Directory -Path $InstallDir -Force | Out-Null
    $exe = Join-Path $tmpDir 'ministr.exe'
    if (-not (Test-Path $exe)) { Write-Err "ministr.exe not found in archive" }
    Move-Item -Path $exe -Destination (Join-Path $InstallDir 'ministr.exe') -Force

    Write-Info "Installed ministr to $InstallDir\ministr.exe"
} finally {
    Remove-Item -Recurse -Force $tmpDir -ErrorAction SilentlyContinue
}

# PATH update via the per-user registry value. Same surface that
# `ministr setup` and the Tauri NSIS installer hook target, so re-runs
# are idempotent. No admin rights required.
$userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
$pathParts = if ($userPath) { $userPath.Split(';') } else { @() }
if ($pathParts -notcontains $InstallDir) {
    $newPath = if ($userPath) { "$userPath;$InstallDir" } else { $InstallDir }
    [Environment]::SetEnvironmentVariable('Path', $newPath, 'User')
    Write-Info "Added $InstallDir to your User PATH."
    Write-Host ""
    Write-Host "Open a new terminal for the PATH change to take effect."
} else {
    Write-Info "$InstallDir is already on your User PATH."
}
