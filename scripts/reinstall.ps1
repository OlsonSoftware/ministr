#Requires -Version 5.1
# Windows counterpart of the `reinstall` just recipe.
#
# Mirrors the macOS/Linux [unix] reinstall in full: kill any running ministr
# processes, clean + rebuild the CLI *and* the Tauri app in release, install
# them into canonical dev locations under %USERPROFILE%\.ministr, and relaunch
# the tray app.
#
# Windows-specific differences from the Unix recipe:
#   - Tauri app install target is %USERPROFILE%\.ministr\app\ (no /Applications
#     analogue on Windows; a dev-owned dir parallels the dev-owned ~/.ministr
#     convention we already use for the CLI).
#   - No codesign step — Windows exes built locally run without re-signing.
#   - Stale socket/pid cleanup: socket doesn't apply (the daemon uses named
#     pipes on Windows, which vanish with the owning process); pid file is
#     still swept for parity with the Unix flow.
#   - No "build a bundle from scratch" fallback is needed. The macOS recipe
#     must clone the sealed, notarized /Applications .app bundle (you cannot
#     write inside it) and so depends on one already existing — which is why
#     scripts/reinstall.sh grew a `tauri build --bundles app` fallback for a
#     clean machine. Windows has no sealed-bundle concept: this recipe always
#     `cargo build`s ministr-app.exe and installs it bare into
#     %USERPROFILE%\.ministr\app\, so it is ALREADY self-sufficient on a clean
#     machine — no released installer prerequisite. The app embeds its
#     resources (icon, etc.) into the exe, so the bare install is complete.
#     (Higher-fidelity dev installs — a real NSIS-installed layout via
#     `tauri build` — would be a separate enhancement, not a clean-machine fix.)

$ErrorActionPreference = 'Stop'

# Abort on non-zero exit from the most recent native command.
# Intentionally NOT a wrapper that takes the command as args, because
# PowerShell advanced-function parameter binding prefix-matches `-p` to
# `-PipelineVariable`, which collides with cargo's `-p <package>` flag.
function Assert-LastExitOk {
    param([string]$What)
    if ($LASTEXITCODE -ne 0) { throw "$What failed (exit $LASTEXITCODE)" }
}

$repoRoot      = Split-Path -Parent $PSScriptRoot
$dataDir       = Join-Path $env:USERPROFILE '.ministr'
$binDir        = Join-Path $dataDir 'bin'
$binPath       = Join-Path $binDir 'ministr.exe'
$appDir        = Join-Path $dataDir 'app'
$appExePath    = Join-Path $appDir 'ministr-app.exe'
$appCliPath    = Join-Path $appDir 'ministr-cli.exe'
$tauriRoot     = Join-Path $repoRoot 'ministr-app'
$tauriSrc      = Join-Path $tauriRoot 'src-tauri'
$tauriIcons    = Join-Path $tauriSrc  'icons'
$tauriBinaries = Join-Path $tauriSrc  'binaries'
$tauriDist     = Join-Path $tauriRoot 'dist'

# Host target triple drives the sidecar filename Tauri's externalBin
# convention expects (e.g. `ministr-cli-x86_64-pc-windows-msvc.exe`).
$hostTriple = (& rustc -vV) | Where-Object { $_ -match '^host:' } |
    ForEach-Object { ($_ -split '\s+', 2)[1].Trim() }
if (-not $hostTriple) { throw 'could not determine rustc host triple — is rustup/rustc on PATH?' }
$sidecarExe = Join-Path $tauriBinaries "ministr-cli-$hostTriple.exe"

# Stop-and-wait helper. Windows blocks overwriting a *running* .exe, so
# we have to verify the process is actually gone before we attempt the
# Copy-Item further down. Mirrors wait_for_exit() in scripts/reinstall.sh.
function Stop-MinistrAnd-Wait {
    Get-Process -Name 'ministr-app', 'ministr' -ErrorAction SilentlyContinue |
        Stop-Process -Force -ErrorAction SilentlyContinue
    $deadline = (Get-Date).AddSeconds(10)
    while ((Get-Date) -lt $deadline) {
        $still = Get-Process -Name 'ministr-app', 'ministr' -ErrorAction SilentlyContinue
        if (-not $still) { return }
        Start-Sleep -Milliseconds 250
    }
    Write-Warning 'ministr-app / ministr still alive after Stop-Process — rename-aside fallback in install step will handle it'
}

# Copy a fresh file over a (possibly running) target. Windows blocks
# overwriting a running .exe with a plain Copy-Item, but it *does* allow
# renaming it — exactly the trick refresh_shadowing_binaries() uses in
# ministr-cli/src/commands.rs. So on a plain-copy failure we move the
# locked file aside and copy the new bytes into place; the leftover
# .stale orphan is best-effort swept here too.
function Install-Atomic {
    param(
        [Parameter(Mandatory)][string]$Source,
        [Parameter(Mandatory)][string]$Destination
    )
    try {
        Copy-Item -Force -Path $Source -Destination $Destination -ErrorAction Stop
        return
    } catch {
        Write-Host "   $Destination is locked — moving aside and replacing"
    }
    $aside = "$Destination.stale"
    Remove-Item -Force -ErrorAction SilentlyContinue $aside
    Move-Item -Force -ErrorAction Stop -Path $Destination -Destination $aside
    Copy-Item -Force -ErrorAction Stop -Path $Source -Destination $Destination
    Remove-Item -Force -ErrorAction SilentlyContinue $aside
}

# Stale socket file only exists on Unix; on Windows the daemon uses named
# pipes which are refcounted kernel objects and disappear on process exit.
# PID file cleanup runs on both platforms.
Remove-Item -Force -ErrorAction SilentlyContinue (Join-Path $dataDir 'ministrd.sock')
Remove-Item -Force -ErrorAction SilentlyContinue (Join-Path $dataDir 'ministrd.pid')

Write-Host '==> Clean rebuild (release)...'
& cargo clean -p ministr-mcp -p ministr-cli -p ministr-daemon -p ministr-app
Assert-LastExitOk 'cargo clean'
# --features directml turns on fastembed's DirectML execution provider so
# embedding / indexing runs on the GPU (any DX12 card — NVIDIA, AMD,
# Intel, Qualcomm). The feature is a no-op on non-Windows, and the code
# falls back to CPU ONNX at runtime if DirectML fails to initialize.
& cargo build --release -p ministr-cli --features directml
Assert-LastExitOk 'cargo build (ministr-cli)'

Write-Host "==> Installing CLI to $binPath (canonical dev location)..."
# Stop here (post-build, immediately before the install steps) so nothing
# has had 30+ seconds of build time to respawn before we replace the
# binaries. Mirrors scripts/reinstall.sh.
Write-Host '   stopping any running ministr processes first...'
Stop-MinistrAnd-Wait

# Legacy/duplicate install roots (~/.cargo\bin, %LOCALAPPDATA%\ministr)
# are no longer cleaned here — `ministr setup` below is the single
# source of truth: it de-PATHs and refreshes every stale shadow.
New-Item -ItemType Directory -Force -Path $binDir | Out-Null
Install-Atomic -Source 'target\release\ministr.exe' -Destination $binPath

# Hand off PATH wiring to `ministr setup`, which uses the onpath crate to
# write HKCU\Environment\PATH and broadcast WM_SETTINGCHANGE. Idempotent —
# re-runs of this dev recipe won't duplicate the entry. Existing shells
# still need to be restarted to pick up the change (Win32 env-block copy
# semantics — no API can change that for already-running processes).
#
# Non-fatal: the binary is already at $binPath either way, so PATH-wiring
# trouble shouldn't abort the rest of the reinstall (Tauri app build,
# tray launch, etc.). Wrapped in try/catch because `$ErrorActionPreference
# = 'Stop'` at the top of this script would otherwise throw on a launch
# failure (missing runtime, AV quarantine, etc.) and skip the fallback
# message entirely. We want both non-zero exits AND launch failures to
# fall through to the manual hint.
Write-Host '==> Adding ministr to PATH via `ministr setup`...'
$setupLaunchError = $null
try {
    & $binPath setup
} catch {
    $setupLaunchError = $_.Exception.Message
}
if ($setupLaunchError -or $LASTEXITCODE -ne 0) {
    if ($setupLaunchError) {
        Write-Warning "ministr setup failed to launch: $setupLaunchError — PATH not updated."
    } else {
        Write-Warning "ministr setup exited $LASTEXITCODE — PATH not updated."
    }
    Write-Host "   Add manually with: [Environment]::SetEnvironmentVariable('Path', `"$binDir;`" + [Environment]::GetEnvironmentVariable('Path','User'), 'User')" -ForegroundColor Yellow
}

# ---- Tauri desktop app ------------------------------------------------------

Write-Host '==> Preparing Tauri app build...'

# 1. Stage the sidecar CLI Tauri's externalBin config points at. Tauri expects
#    the exe at `binaries/ministr-cli-<host-triple>.exe`; we just-built
#    ministr.exe a few steps above, so copy it into place.
New-Item -ItemType Directory -Force -Path $tauriBinaries | Out-Null
Copy-Item -Force -Path 'target\release\ministr.exe' -Destination $sidecarExe

# 2. Ensure the Windows icon exists. Tauri's Win32 resource compiler needs
#    `icons/icon.ico`; generate the full icon set from the source PNG if
#    it's not present yet (idempotent — Tauri just overwrites on re-run).
if (-not (Test-Path (Join-Path $tauriIcons 'icon.ico'))) {
    Write-Host '   generating icon.ico from icon.png...'
    Push-Location $tauriSrc
    try {
        & npx --yes '@tauri-apps/cli' icon 'icons/icon.png' | Out-Null
        Assert-LastExitOk 'tauri icon'
    } finally {
        Pop-Location
    }
}

# 3. Frontend build (Vite). Always sync — checking for node_modules
#    skips a partial install (lockfile drift, interrupted prior run)
#    and leaves vite to fail at build time. `--frozen-lockfile` is a
#    no-op when in sync and fails loudly if package.json and
#    pnpm-lock.yaml disagree.
Write-Host '   syncing frontend dependencies (pnpm install --frozen-lockfile)...'
Push-Location $tauriRoot
try {
    & pnpm install --frozen-lockfile
    Assert-LastExitOk 'pnpm install'
} finally {
    Pop-Location
}

Write-Host '   building frontend (vite build)...'
Push-Location $tauriRoot
try {
    & pnpm run build
    Assert-LastExitOk 'pnpm run build'
} finally {
    Pop-Location
}

# 4. Rust build of the Tauri app itself (release, with DirectML for GPU
#    embedding inside the embedded daemon).
Write-Host '   building Tauri app (cargo release)...'
& cargo build --release -p ministr-app --features directml
Assert-LastExitOk 'cargo build (ministr-app)'

# 5. Install into a canonical dev location (%USERPROFILE%\.ministr\app\).
#    The Tauri app's setup.rs discovers its sidecar next to the main exe,
#    so we keep ministr-cli.exe as a sibling just like the macOS .app
#    bundle's Contents/MacOS/ layout.
Write-Host "==> Installing Tauri app to $appDir..."
New-Item -ItemType Directory -Force -Path $appDir | Out-Null
Install-Atomic -Source (Join-Path $repoRoot 'target\release\ministr-app.exe') -Destination $appExePath
Install-Atomic -Source (Join-Path $repoRoot 'target\release\ministr.exe')     -Destination $appCliPath

# 6. Launch the freshly-installed app.
Write-Host '==> Launching tray app...'
Start-Process -FilePath $appExePath -WorkingDirectory $appDir | Out-Null

Write-Host '==> Done. Restart your Claude Code session to pick up the new binary.'
