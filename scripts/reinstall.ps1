#Requires -Version 5.1
# Windows counterpart of the `reinstall` just recipe.
#
# Mirrors the macOS/Linux [unix] reinstall in full: kill any running ministr
# processes, clean + rebuild the CLI in release, and install it into the
# canonical dev location under %USERPROFILE%\.ministr.

$ErrorActionPreference = 'Stop'

# Abort on non-zero exit from the most recent native command.
# Intentionally NOT a wrapper that takes the command as args, because
# PowerShell advanced-function parameter binding prefix-matches `-p` to
# `-PipelineVariable`, which collides with cargo's `-p <package>` flag.
function Assert-LastExitOk {
    param([string]$What)
    if ($LASTEXITCODE -ne 0) { throw "$What failed (exit $LASTEXITCODE)" }
}

$repoRoot = Split-Path -Parent $PSScriptRoot
$dataDir  = Join-Path $env:USERPROFILE '.ministr'
$binDir   = Join-Path $dataDir 'bin'
$binPath  = Join-Path $binDir 'ministr.exe'

# Stop-and-wait helper. Windows blocks overwriting a *running* .exe, so
# we have to verify the process is actually gone before we attempt the
# Copy-Item further down. Mirrors wait_for_exit() in scripts/reinstall.sh.
# 'ministr-app' is the legacy desktop binary (removed in the GUI v8 reset,
# 2026-08) — still stopped here so a stale install can't hold a lock.
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
& cargo clean -p ministr-mcp -p ministr-cli -p ministr-daemon
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
# trouble shouldn't abort the rest of the reinstall. Wrapped in try/catch
# because `$ErrorActionPreference = 'Stop'` at the top of this script would
# otherwise throw on a launch failure (missing runtime, AV quarantine, etc.)
# and skip the fallback message entirely. We want both non-zero exits AND
# launch failures to fall through to the manual hint.
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

Write-Host '==> Done. Restart your Claude Code session to pick up the new binary.'
