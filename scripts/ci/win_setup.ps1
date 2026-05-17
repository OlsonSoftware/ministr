<#
.SYNOPSIS
  Idempotent bootstrap for the self-hosted Windows release runner.

  Why this exists: a self-hosted Windows runner has no guaranteed
  toolchain, and `shell: bash` there resolves to the System32 WSL stub
  (exits 1 with no distro). So the Windows release path uses ZERO bash:
  this script (Windows PowerShell 5.1 - always present, no WSL, no pwsh
  dependency) guarantees Python + the Rust toolchain, and everything
  after it is Python (scripts/ci/ci.py). dtolnay/rust-toolchain is
  skipped on Windows precisely because its internal step is `shell:
  bash`.

  Safe to re-run: every action is guarded (install only if missing).

.PARAMETER Target
  Rust target triple to ensure is installed (e.g. x86_64-pc-windows-msvc).
#>
param([Parameter(Mandatory = $true)][string]$Target)

$ErrorActionPreference = 'Stop'
function Have($name) { $null -ne (Get-Command $name -ErrorAction SilentlyContinue) }

# Print a tool's version WITHOUT ever aborting the script. Under
# $ErrorActionPreference='Stop', PowerShell 5.1 turns a native
# command's stderr write into a *terminating* NativeCommandError when
# captured (rustup --version writes an "info:" line to stderr; even
# `2>$null` did not reliably suppress the throw). Force 'Continue' for
# the duration and swallow anything: these prints are diagnostic only.
function Show-Version([string]$label, [scriptblock]$cmd) {
  $prev = $ErrorActionPreference
  $ErrorActionPreference = 'Continue'
  try {
    $v = (& $cmd 2>&1 | Out-String).Trim()
    Write-Host "${label}: $v"
  } catch {
    Write-Host "${label}: (version check skipped: $($_.Exception.Message))"
  } finally {
    $ErrorActionPreference = $prev
  }
}

Write-Host "== Windows runner bootstrap (target=$Target) =="

# --- Python -------------------------------------------------------------
# Do NOT use winget OR the python.org installer. This runner is a
# locked-down, service-account, frequently-reset Windows VM:
#   * winget is absent (not on Server / fresh dev images; manual
#     installs are wiped on reset);
#   * the python.org .exe is an MSI bootstrapper -> fails with 1601
#     ("Windows Installer service could not be accessed") under a
#     non-interactive service account.
# Use python-build-standalone: a fully self-contained CPython that is
# just EXTRACTED (no installer, no MSI, no admin) and already ships
# pip. Idempotent: cached under USERPROFILE, only fetched if missing.
$PyVersion = '3.12.7'
$PbsTag    = '20241016'
if (Have 'python') {
  Show-Version 'python' { python --version }
} else {
  $pyHome = Join-Path $env:USERPROFILE ".python-standalone\$PyVersion-$PbsTag"
  $pyDir  = Join-Path $pyHome 'python'
  $pyBin  = Join-Path $pyDir 'python.exe'
  if (-not (Test-Path $pyBin)) {
    Write-Host "python: not found - fetching python-build-standalone $PyVersion ($PbsTag)"
    $tgz = Join-Path $env:RUNNER_TEMP 'python-standalone.tar.gz'
    $url = "https://github.com/astral-sh/python-build-standalone/releases/download/$PbsTag/cpython-$PyVersion+$PbsTag-x86_64-pc-windows-msvc-install_only.tar.gz"
    Invoke-WebRequest -Uri $url -OutFile $tgz -UseBasicParsing
    New-Item -ItemType Directory -Force -Path $pyHome | Out-Null
    # Windows ships bsdtar as tar.exe; it extracts .tar.gz natively.
    # The install_only archive unpacks to a top-level 'python\' dir.
    tar -xf $tgz -C $pyHome
    if ($LASTEXITCODE -ne 0) { throw "tar extract failed ($LASTEXITCODE)" }
    if (-not (Test-Path $pyBin)) { throw 'python-build-standalone layout unexpected (no python\python.exe)' }
  }
  # Extract changes no PATH; prepend for this process + persist via
  # GITHUB_PATH (ASCII: PowerShell 5.1 mis-decodes a UTF-8 BOM here).
  $env:PATH = "$pyDir;$pyDir\Scripts;$env:PATH"
  if ($env:GITHUB_PATH) {
    "$pyDir"         | Out-File -FilePath $env:GITHUB_PATH -Encoding ascii -Append
    "$pyDir\Scripts" | Out-File -FilePath $env:GITHUB_PATH -Encoding ascii -Append
  }
  if (-not (Have 'python')) { throw 'python still not on PATH after extract' }
  Show-Version 'python' { python --version }
}

# --- Rust (rustup) ------------------------------------------------------
if (-not (Have 'rustup')) {
  Write-Host 'rustup: not found - installing'
  $ri = Join-Path $env:RUNNER_TEMP 'rustup-init.exe'
  Invoke-WebRequest -Uri 'https://win.rustup.rs/x86_64' -OutFile $ri -UseBasicParsing
  & $ri -y --default-toolchain stable --profile minimal --no-modify-path
  if ($LASTEXITCODE -ne 0) { throw "rustup-init failed ($LASTEXITCODE)" }
  $cargoBin = Join-Path $env:USERPROFILE '.cargo\bin'
  $env:PATH = "$cargoBin;$env:PATH"
  "$cargoBin" | Out-File -FilePath $env:GITHUB_PATH -Encoding utf8 -Append
}
Show-Version 'rustup' { rustup --version }

# Ensure stable + the requested target (idempotent - rustup is a no-op
# if already present/up to date).
rustup toolchain install stable --profile minimal --no-self-update
if ($LASTEXITCODE -ne 0) { throw "rustup toolchain install failed ($LASTEXITCODE)" }
rustup default stable
rustup target add $Target
if ($LASTEXITCODE -ne 0) { throw "rustup target add $Target failed ($LASTEXITCODE)" }

Show-Version 'rustc' { rustc --version }

# --- Defender exclusions (idempotent; folded in from the old separate
#     workflow step so all Windows prep lives in one script). Non-fatal:
#     a runner without Defender / without admin must not break the build.
try {
  $work = $env:GITHUB_WORKSPACE
  foreach ($p in @($work, "$env:USERPROFILE\.cargo", "$env:USERPROFILE\.rustup", $env:RUNNER_TEMP)) {
    if ($p) { Add-MpPreference -ExclusionPath $p -ErrorAction Stop }
  }
  foreach ($x in 'rustc.exe','cargo.exe','link.exe','lld-link.exe','sccache.exe','python.exe') {
    Add-MpPreference -ExclusionProcess $x -ErrorAction Stop
  }
  Write-Host 'Defender exclusions applied.'
} catch {
  Write-Host "Defender exclusions skipped (non-fatal): $($_.Exception.Message)"
}

# --- sccache + R2 wiring (parity with the Linux/macOS rust-env action)
#     so Windows release compiles are warm-cached too. Secrets arrive
#     via the step `env:` block; absent -> sccache falls back to a local
#     on-disk cache (harmless).
if (-not (Have 'sccache')) {
  Write-Host 'sccache: not found - installing'
  if (Have 'cargo-binstall') {
    cargo binstall --no-confirm --no-symlinks sccache
  } else {
    cargo install sccache --locked
  }
}
Show-Version 'sccache' { sccache --version }
if ($env:GITHUB_ENV) {
  Add-Content -Path $env:GITHUB_ENV -Value 'RUSTC_WRAPPER=sccache'
  Add-Content -Path $env:GITHUB_ENV -Value 'SCCACHE_REGION=auto'
  if ($env:SCCACHE_BUCKET) {
    Add-Content -Path $env:GITHUB_ENV -Value "SCCACHE_BUCKET=$env:SCCACHE_BUCKET"
    Add-Content -Path $env:GITHUB_ENV -Value "SCCACHE_ENDPOINT=$env:SCCACHE_ENDPOINT"
    Add-Content -Path $env:GITHUB_ENV -Value "AWS_ACCESS_KEY_ID=$env:AWS_ACCESS_KEY_ID"
    Add-Content -Path $env:GITHUB_ENV -Value "AWS_SECRET_ACCESS_KEY=$env:AWS_SECRET_ACCESS_KEY"
    Write-Host "sccache -> R2 bucket '$env:SCCACHE_BUCKET'"
  } else {
    Write-Host 'sccache -> local on-disk cache (no R2 secrets)'
  }
}

Write-Host '== bootstrap OK =='
