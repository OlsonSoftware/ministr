#!/usr/bin/env bash
# Unix counterpart of the `reinstall` just recipe (macOS + Linux).
#
# Mirrors scripts/reinstall.ps1 in structure: clean + rebuild the CLI and
# Tauri app in release, stop running instances, replace the installed
# binaries atomically, then relaunch the tray.
#
# The kill-and-replace dance is the load-bearing part: we cannot overwrite
# a *running* signed Mach-O on macOS (kernel returns EPERM even with sudo),
# and on Linux ETXTBSY can bite for similar reasons. The fix is to (1)
# stop everything immediately before the install (not 30+ seconds earlier
# at the top of the build), and (2) use atomic rename — the kernel keeps
# the old inode alive for the running process, while we swap the directory
# entry to point at the fresh binary. This mirrors refresh_shadowing_binaries
# in ministr-cli/src/commands.rs which solves the same problem on Windows.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$REPO_ROOT"

case "$(uname -s)" in
    Darwin) OS="macos" ;;
    Linux)  OS="linux" ;;
    *) echo "Unsupported OS: $(uname -s)" >&2; exit 1 ;;
esac

# ─── helpers ───────────────────────────────────────────────────────────────────

# Unload any per-user launchd agent whose plist mentions ministr, so the
# kernel can't race-respawn ministr-app between our SIGTERM and the rename.
# Glob match catches the canonical plist plus any leftover labels from
# old bundle ids (e.g. com.ministr.desktop.plist) without us having to
# enumerate them. Best-effort: bootout is non-fatal here.
bootout_ministr_agents() {
    [ -d "$HOME/Library/LaunchAgents" ] || return 0
    local uid plist label
    uid="$(id -u)"
    for plist in "$HOME"/Library/LaunchAgents/*ministr*.plist; do
        [ -f "$plist" ] || continue
        label="$(basename "$plist" .plist)"
        launchctl bootout "gui/$uid/$label" 2>/dev/null || true
    done
}

# Stop a systemd --user unit only if it's actually loaded; never fail.
stop_systemd_user_unit() {
    local unit="$1"
    command -v systemctl >/dev/null 2>&1 || return 0
    systemctl --user list-unit-files --no-legend "$unit" 2>/dev/null \
        | awk '{print $1}' | grep -qx "$unit" || return 0
    systemctl --user stop "$unit" 2>/dev/null || true
}

# Poll until no process matches `$1` (full-command-line), or `$2` seconds
# elapse. Returns 0 if exited, 1 if still alive at the timeout.
wait_for_exit() {
    local pattern="$1"
    local timeout_s="${2:-10}"
    local deadline
    deadline=$(( $(date +%s) + timeout_s ))
    while [ "$(date +%s)" -lt "$deadline" ]; do
        if ! pgrep -f "$pattern" >/dev/null 2>&1; then
            return 0
        fi
        sleep 0.25
    done
    return 1
}

stop_ministr() {
    echo "==> Stopping any running ministr instances..."

    if [ "$OS" = "macos" ]; then
        bootout_ministr_agents
    elif [ "$OS" = "linux" ]; then
        stop_systemd_user_unit "ministr-desktop.service"
        stop_systemd_user_unit "ai.ministr.desktop.service"
    fi

    # Polite shutdown first so the app can flush state.
    pkill -TERM -f "ministr-app"   2>/dev/null || true
    pkill -TERM -f "ministr serve" 2>/dev/null || true
    pkill -TERM -f "ministr __daemon" 2>/dev/null || true

    if ! wait_for_exit "ministr-app" 5; then
        echo "   SIGTERM didn't drop ministr-app within 5s — sending SIGKILL"
        pkill -KILL -f "ministr-app" 2>/dev/null || true
    fi
    pkill -KILL -f "ministr serve"    2>/dev/null || true
    pkill -KILL -f "ministr __daemon" 2>/dev/null || true

    if pgrep -f "ministr-app" >/dev/null 2>&1; then
        echo "   WARNING: ministr-app still running after SIGKILL — atomic rename will proceed anyway" >&2
    fi

    rm -f "$HOME/.ministr/ministrd.sock" "$HOME/.ministr/ministrd.pid"
}

# Atomic in-place replace. Stages the new binary at `<dst>.new` in the
# same directory (so rename(2) is atomic — same filesystem), `chmod`s it
# executable, then `mv -f`s it over the target. The rename swaps only
# the directory entry, leaving the running process's mapped inode intact,
# which is why this works even when the target is currently executing.
atomic_install() {
    local src="$1"
    local dst="$2"
    local sudo_cmd="${3:-}"
    local staged="${dst}.new"

    $sudo_cmd cp -f "$src" "$staged"
    $sudo_cmd chmod 755 "$staged"
    $sudo_cmd mv -f "$staged" "$dst"
}

# ─── build ─────────────────────────────────────────────────────────────────────

echo "==> Clean rebuild (release)..."
cargo clean -p ministr-mcp -p ministr-cli -p ministr-daemon -p ministr-app
cargo build --release -p ministr-cli

# Tauri's externalBin (tauri.conf.json) requires the sidecar at
# `ministr-app/src-tauri/binaries/ministr-cli-<host-triple>` before
# the ministr-app build script runs. Mirrors scripts/reinstall.ps1.
HOST_TRIPLE="$(rustc -vV | awk '/^host:/ { print $2 }')"
mkdir -p ministr-app/src-tauri/binaries
cp target/release/ministr "ministr-app/src-tauri/binaries/ministr-cli-${HOST_TRIPLE}"

# Tauri's `generate_context!` proc macro reads `frontendDist` from
# tauri.conf.json (`../dist`) at compile time, so the Vite output must
# exist before `cargo build -p ministr-app`. `tauri build` would run
# beforeBuildCommand for us; raw cargo doesn't. Always sync — checking
# for `node_modules` skips a partial install (lockfile drift, interrupted
# prior run) and leaves vite to fail at build time. `--frozen-lockfile`
# is a no-op when in sync and fails loudly if package.json and
# pnpm-lock.yaml disagree.
echo "==> Syncing frontend deps (pnpm install --frozen-lockfile)..."
( cd ministr-app && pnpm install --frozen-lockfile )
echo "==> Building frontend (vite)..."
( cd ministr-app && pnpm run build )
cargo build --release -p ministr-app

# ─── install CLI ──────────────────────────────────────────────────────────────

echo "==> Installing CLI to ~/.ministr/bin/ministr (canonical dev location)..."
# Remove stale copies from other locations to prevent shadow binaries.
rm -f "$HOME/.cargo/bin/ministr"
rm -f /usr/local/bin/ministr 2>/dev/null || true
mkdir -p "$HOME/.ministr/bin"
# CLI isn't typically running as a long-lived daemon under this path, but
# use atomic_install anyway for parity — same cost, removes a foot-gun.
atomic_install target/release/ministr "$HOME/.ministr/bin/ministr"

# Hand off PATH wiring to `ministr setup` (onpath crate). Detects
# installed shells and writes the right rc-file edits. Idempotent —
# re-runs of this dev recipe won't duplicate entries. Non-fatal: the
# binary is at ~/.ministr/bin/ministr regardless, so PATH-wiring trouble
# shouldn't abort the rest of the reinstall.
echo "==> Adding ministr to PATH via \`ministr setup\`..."
if ! "$HOME/.ministr/bin/ministr" setup; then
    echo "   ministr setup failed — add manually with:" >&2
    echo "     export PATH=\"\$HOME/.ministr/bin:\$PATH\"" >&2
fi

# ─── install Tauri app ────────────────────────────────────────────────────────

if [ "$OS" = "macos" ]; then
    # Install to ~/Applications/ministr.app/ rather than /Applications.
    #
    # Why: the released .pkg installs a signed + notarized + hardened-runtime
    # bundle into /Applications, and modern macOS refuses writes inside such
    # a bundle even as root (kernel returns EPERM on file creation — not a
    # SIP xattr, but the bundle-seal enforcement that ships with notarized
    # apps). Stripping the seal would mean ad-hoc-re-signing the user's
    # actual installed app, which we don't want a dev recipe to do.
    #
    # User-owned ~/Applications sidesteps the whole fight: parallels the
    # Windows %USERPROFILE%\.ministr\app\ pattern, no sudo, kernel happy.
    # First run bootstraps the bundle skeleton (Info.plist, Resources/, the
    # sidecar layout setup.rs expects) by cloning from /Applications via
    # ditto — preserves xattrs/metadata better than cp -R.
    RELEASED_BUNDLE="/Applications/ministr.app"
    APP_BUNDLE="$HOME/Applications/ministr.app"
    APP_TARGET="$APP_BUNDLE/Contents/MacOS/ministr-app"
    APP_CLI_TARGET="$APP_BUNDLE/Contents/MacOS/ministr-cli"

    # Stop now (post-build, immediately before install) so the released
    # /Applications instance isn't competing with our dev launch via the
    # single-instance plugin (both bundles share the `ai.ministr.desktop`
    # identifier, so only one can hold the lock).
    stop_ministr

    if [ ! -d "$APP_BUNDLE/Contents/MacOS" ]; then
        if [ ! -d "$RELEASED_BUNDLE/Contents/MacOS" ]; then
            cat >&2 <<EOF
   No ministr.app bundle found at either $APP_BUNDLE or $RELEASED_BUNDLE.
   This recipe only updates the inner binary; it cannot build the
   .app bundle from scratch. Run \`just pkg-dev\` (or \`just pkg\` for
   a signed+notarized build), install the produced .pkg, then
   re-run this recipe — the .pkg provides the bundle skeleton this
   recipe clones into ~/Applications.
EOF
            exit 1
        fi
        echo "==> First-run bootstrap: cloning bundle skeleton from $RELEASED_BUNDLE..."
        mkdir -p "$HOME/Applications"
        # ditto preserves resource forks, xattrs, ACLs better than cp -R.
        # The clone goes to a user-owned path so subsequent writes don't
        # need sudo and don't hit the notarized-bundle seal.
        ditto "$RELEASED_BUNDLE" "$APP_BUNDLE"
    fi

    echo "==> Installing Tauri app to $APP_BUNDLE (atomic replace)..."
    atomic_install target/release/ministr-app "$APP_TARGET"
    # Sidecar binary lives inside the bundle too; keep it in sync.
    if [ -f "$APP_CLI_TARGET" ]; then
        atomic_install target/release/ministr "$APP_CLI_TARGET"
    fi

    # We replaced two binaries inside the bundle, which invalidates the
    # original Developer ID seal. Ad-hoc re-sign so Gatekeeper/launchd
    # will accept it (ad-hoc is fine for a locally-owned dev bundle —
    # the `-` identity skips notarization checks for `open` from this
    # session). No sudo needed since the bundle is user-owned.
    codesign --force --deep --sign - "$APP_BUNDLE" >/dev/null 2>&1 || true

    # We don't reload `ai.ministr.desktop.plist` here — it points at the
    # /Applications/ministr.app path (installed by setup.rs::install_launchd_plist),
    # so at next login it will start the released bundle again, not this
    # dev one. That's the right default: a dev `just reinstall` shouldn't
    # silently change what your machine auto-launches at login. Re-run
    # the recipe in any session to bring the dev bundle back up.

    echo "==> Launching tray app from $APP_BUNDLE..."
    open "$APP_BUNDLE"

elif [ "$OS" = "linux" ]; then
    # Honour whatever the user installed: .deb/.rpm → /usr/bin/ministr-app,
    # local build → ~/.ministr/app/, etc. Fall back to ~/.ministr/app/ if
    # nothing is on PATH yet, paralleling the Windows script's dev-only target.
    APP_TARGET="$(command -v ministr-app 2>/dev/null || true)"
    if [ -n "$APP_TARGET" ] && [ -L "$APP_TARGET" ]; then
        # Follow symlinks so we overwrite the file, not the link.
        APP_TARGET="$(readlink -f "$APP_TARGET")"
    fi
    if [ -z "$APP_TARGET" ]; then
        APP_TARGET="$HOME/.ministr/app/ministr-app"
        echo "   no installed ministr-app on PATH; falling back to $APP_TARGET"
        mkdir -p "$(dirname "$APP_TARGET")"
    fi

    # Stop now (post-build, immediately before install).
    stop_ministr

    SUDO=""
    if [ -e "$APP_TARGET" ] && [ ! -w "$APP_TARGET" ]; then
        SUDO="sudo"
        echo "   target $APP_TARGET is not user-writable — using sudo"
    elif [ ! -e "$APP_TARGET" ] && [ ! -w "$(dirname "$APP_TARGET")" ]; then
        SUDO="sudo"
        echo "   target directory $(dirname "$APP_TARGET") is not user-writable — using sudo"
    fi

    echo "==> Installing Tauri app to $APP_TARGET (atomic replace)..."
    atomic_install target/release/ministr-app "$APP_TARGET" "$SUDO"

    # If a systemd user unit existed, bring it back.
    for unit in ministr-desktop.service ai.ministr.desktop.service; do
        if command -v systemctl >/dev/null 2>&1 \
            && systemctl --user list-unit-files --no-legend "$unit" 2>/dev/null \
                | awk '{print $1}' | grep -qx "$unit"; then
            systemctl --user start "$unit" 2>/dev/null || true
            break
        fi
    done

    echo "==> Launching tray app..."
    # `setsid` + redirected stdio so the new tray survives this shell exiting.
    # `setsid` is part of util-linux on every mainstream distro; `nohup` is
    # a portable fallback if it's not present.
    if command -v setsid >/dev/null 2>&1; then
        setsid "$APP_TARGET" >/dev/null 2>&1 < /dev/null &
    else
        nohup "$APP_TARGET" >/dev/null 2>&1 < /dev/null &
    fi
    disown 2>/dev/null || true
fi

echo "==> Done. Restart your Claude Code session to pick up the new binary."
