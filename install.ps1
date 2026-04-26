# ministr installer for Windows — downloads the latest release binary
# from our release proxy and adds it to PATH.
#
# Usage:
#   iwr -useb https://ministr.app/install.ps1 | iex
#
# Mirrors install.sh: fetches assets from https://dl.ministr.app, a
# Cloudflare Worker fronting the private GitHub repo's releases. All
# downloads are unauthenticated HTTPS GETs.
#
# Honors the same env-var contract as install.sh:
#   MINISTR_DL_HOST  override the download host (testing / mirrors)
#   INSTALL_DIR      override the install location
#                    (default: %USERPROFILE%\.ministr\bin)

[CmdletBinding()]
param()

$ErrorActionPreference = 'Stop'

# Windows PowerShell 5.1 defaults to TLS 1.0/1.1 which github.com (and
# the Cloudflare proxy) reject. PowerShell 7+ already negotiates TLS
# 1.2+, but force it here too — cheap and idempotent.
[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12

function Write-Info { param([string]$Msg) Write-Host $Msg -ForegroundColor Blue }
function Write-Err  { param([string]$Msg) Write-Host "error: $Msg" -ForegroundColor Red; exit 1 }

$DlHost     = if ($env:MINISTR_DL_HOST) { $env:MINISTR_DL_HOST } else { 'https://dl.ministr.app' }
$InstallDir = if ($env:INSTALL_DIR)     { $env:INSTALL_DIR }     else { Join-Path $env:USERPROFILE '.ministr\bin' }

# Detect architecture. We only ship x86_64 Windows today; aarch64 maps
# to a target triple we don't build for yet, so fail loudly rather than
# fetch an asset that doesn't exist.
$arch = switch ($env:PROCESSOR_ARCHITECTURE) {
    'AMD64' { 'x86_64' }
    'ARM64' { Write-Err 'aarch64 Windows is not yet supported (no release artifact). Build from source: cargo install --path ministr-cli' }
    default { Write-Err "unsupported architecture: $env:PROCESSOR_ARCHITECTURE" }
}

$target  = "$arch-pc-windows-msvc"
$archive = "ministr-$target.zip"

Write-Info 'Finding latest ministr release...'
try {
    $latest = Invoke-RestMethod -Uri "$DlHost/latest" -UseBasicParsing
} catch {
    Write-Err "could not reach $DlHost/latest — check your network or set MINISTR_DL_HOST. ($_)"
}
$tag = $latest.tag
if (-not $tag) { Write-Err "could not determine latest release tag from $DlHost/latest" }
Write-Info "Latest release: $tag"

$url = "$DlHost/$tag/$archive"

Write-Info "Downloading $archive..."
$tmpDir = Join-Path ([IO.Path]::GetTempPath()) ("ministr-install-" + [guid]::NewGuid().ToString('N').Substring(0, 8))
New-Item -ItemType Directory -Force -Path $tmpDir | Out-Null
try {
    $zipPath = Join-Path $tmpDir $archive
    Invoke-WebRequest -Uri $url -OutFile $zipPath -UseBasicParsing

    Expand-Archive -Path $zipPath -DestinationPath $tmpDir -Force

    New-Item -ItemType Directory -Force -Path $InstallDir | Out-Null
    $exeSrc = Join-Path $tmpDir 'ministr.exe'
    if (-not (Test-Path $exeSrc)) {
        Write-Err "expected ministr.exe inside $archive but it wasn't there. Asset layout may have changed."
    }
    $exeDst = Join-Path $InstallDir 'ministr.exe'
    Move-Item -Force -Path $exeSrc -Destination $exeDst

    Write-Info "Installed ministr to $exeDst"
} finally {
    Remove-Item -Recurse -Force -ErrorAction SilentlyContinue $tmpDir
}

# Add InstallDir to per-user PATH (HKCU\Environment) idempotently.
# Using [Environment]::SetEnvironmentVariable rather than `setx` because
# setx truncates PATH at 1024 chars on stock Windows — long PATHs are
# common on dev machines and silent truncation is the worst kind of
# regression to debug after the fact.
$normalizedTarget = $InstallDir.TrimEnd('\', '/').ToLowerInvariant()
$currentUserPath  = [Environment]::GetEnvironmentVariable('Path', 'User')
$alreadyOnPath = $false
if ($currentUserPath) {
    $alreadyOnPath = ($currentUserPath -split ';') |
        Where-Object { $_ -and ($_.TrimEnd('\', '/').ToLowerInvariant() -eq $normalizedTarget) } |
        Select-Object -First 1
}

if ($alreadyOnPath) {
    Write-Info "$InstallDir is already on your User PATH."
} else {
    $newUserPath = if ($currentUserPath) { "$InstallDir;$currentUserPath" } else { $InstallDir }
    [Environment]::SetEnvironmentVariable('Path', $newUserPath, 'User')
    Write-Info "Added $InstallDir to your User PATH."

    # Broadcast WM_SETTINGCHANGE so newly-spawned processes (e.g. fresh
    # terminals) pick up the new PATH without requiring a logout. The
    # current process's $env:Path is a copy and won't update — users
    # have to open a new terminal regardless. SendMessageTimeout with
    # SMTO_ABORTIFHUNG + a short timeout is the documented pattern;
    # see Microsoft docs on environment variable propagation.
    if (-not ('Win32.NativeMethods' -as [type])) {
        Add-Type -Namespace Win32 -Name NativeMethods -MemberDefinition @'
[System.Runtime.InteropServices.DllImport("user32.dll", SetLastError = true, CharSet = System.Runtime.InteropServices.CharSet.Auto)]
public static extern System.IntPtr SendMessageTimeout(System.IntPtr hWnd, uint Msg, System.UIntPtr wParam, string lParam, uint fuFlags, uint uTimeout, out System.UIntPtr lpdwResult);
'@
    }
    $HWND_BROADCAST    = [IntPtr]0xffff
    $WM_SETTINGCHANGE  = 0x1A
    $SMTO_ABORTIFHUNG  = 0x2
    $result = [UIntPtr]::Zero
    [void][Win32.NativeMethods]::SendMessageTimeout($HWND_BROADCAST, $WM_SETTINGCHANGE, [UIntPtr]::Zero, 'Environment', $SMTO_ABORTIFHUNG, 5000, [ref]$result)
}

Write-Host ''
Write-Info 'Open a new terminal and run:'
Write-Host '  ministr --version'
Write-Host ''
