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

Write-Host "== Windows runner bootstrap (target=$Target) =="

# --- Python -------------------------------------------------------------
# NOTE: do NOT depend on winget. This is a self-hosted, frequently-reset
# Windows eval VM; winget is often absent (not on Server / fresh dev
# images, and any manual install is wiped on VM reset). We bootstrap
# from the official python.org per-user installer via Invoke-WebRequest
# (same mechanism already used for rustup below) - no winget, no admin,
# no msstore cert dance. Idempotent: only runs if python is missing.
$PyVersion = '3.12.7'
if (Have 'python') {
  Write-Host "python: $(python --version 2>&1)"
} else {
  Write-Host "python: not found - installing $PyVersion from python.org (winget-free, no admin)"
  $pyExe = Join-Path $env:RUNNER_TEMP "python-$PyVersion-amd64.exe"
  $pyUrl = "https://www.python.org/ftp/python/$PyVersion/python-$PyVersion-amd64.exe"
  Invoke-WebRequest -Uri $pyUrl -OutFile $pyExe -UseBasicParsing
  # Per-user silent install (no admin): installs to
  # %LOCALAPPDATA%\Programs\Python\Python312.
  $p = Start-Process -FilePath $pyExe -Wait -PassThru -ArgumentList @(
    '/quiet', 'InstallAllUsers=0', 'PrependPath=1', 'Include_pip=1',
    'Include_launcher=1', 'Shortcuts=0', 'AssociateFiles=0'
  )
  if ($p.ExitCode -ne 0) { throw "python installer failed ($($p.ExitCode))" }
  # Installer PATH changes don't apply to the current process. Resolve
  # the install dir (standard path first; fall back to a search) and
  # prepend it for this job + persist via GITHUB_PATH.
  $pyDir = Join-Path $env:LOCALAPPDATA 'Programs\Python\Python312'
  if (-not (Test-Path (Join-Path $pyDir 'python.exe'))) {
    $found = Get-ChildItem -Path (Join-Path $env:LOCALAPPDATA 'Programs\Python') `
      -Filter 'python.exe' -Recurse -ErrorAction SilentlyContinue | Select-Object -First 1
    if ($found) { $pyDir = $found.DirectoryName }
  }
  if (Test-Path (Join-Path $pyDir 'python.exe')) {
    $env:PATH = "$pyDir;$pyDir\Scripts;$env:PATH"
    if ($env:GITHUB_PATH) {
      "$pyDir"          | Out-File -FilePath $env:GITHUB_PATH -Encoding utf8 -Append
      "$pyDir\Scripts"  | Out-File -FilePath $env:GITHUB_PATH -Encoding utf8 -Append
    }
  }
  if (-not (Have 'python')) { throw 'python still not on PATH after install' }
  Write-Host "python: $(python --version 2>&1)"
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
Write-Host "rustup: $(rustup --version 2>&1)"

# Ensure stable + the requested target (idempotent - rustup is a no-op
# if already present/up to date).
rustup toolchain install stable --profile minimal --no-self-update
if ($LASTEXITCODE -ne 0) { throw "rustup toolchain install failed ($LASTEXITCODE)" }
rustup default stable
rustup target add $Target
if ($LASTEXITCODE -ne 0) { throw "rustup target add $Target failed ($LASTEXITCODE)" }

Write-Host "rustc: $(rustc --version 2>&1)"

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
Write-Host "sccache: $(sccache --version 2>&1)"
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
