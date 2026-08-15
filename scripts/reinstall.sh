#!/usr/bin/env bash
# Unix counterpart of the `reinstall` just recipe (macOS + Linux).
#
# Mirrors scripts/reinstall.ps1 in structure: clean + rebuild the CLI in
# release, stop running instances, replace the installed binary atomically.
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

# The Tauri desktop app was removed in the GUI v8 reset (2026-08). Machines
# with an older install may still have its launchd agents / bundles — unload
# and delete the stale agents so nothing tries to auto-launch a dead app.
cleanup_legacy_desktop_agents() {
    [ -d "$HOME/Library/LaunchAgents" ] || return 0
    local uid plist label
    uid="$(id -u)"
    for plist in "$HOME"/Library/LaunchAgents/*ministr*.desktop.plist; do
        [ -f "$plist" ] || continue
        label="$(basename "$plist" .plist)"
        launchctl bootout "gui/$uid/$label" 2>/dev/null || true
        rm -f "$plist" && echo "   removed legacy desktop launch agent: $label"
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

stop_ministr() {
    echo "==> Stopping any running ministr instances..."

    if [ "$OS" = "macos" ]; then
        cleanup_legacy_desktop_agents
    elif [ "$OS" = "linux" ]; then
        stop_systemd_user_unit "ministr-desktop.service"
        stop_systemd_user_unit "ai.ministr.desktop.service"
    fi

    # Polite shutdown first so processes can flush state. `ministr-app` is
    # the legacy desktop binary — kill it too if a stale install is running.
    pkill -TERM -f "ministr-app"   2>/dev/null || true
    pkill -TERM -f "ministr serve" 2>/dev/null || true
    pkill -TERM -f "ministr __daemon" 2>/dev/null || true
    sleep 1
    pkill -KILL -f "ministr-app"      2>/dev/null || true
    pkill -KILL -f "ministr serve"    2>/dev/null || true
    pkill -KILL -f "ministr __daemon" 2>/dev/null || true

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

# Canonical dev install location for the `ministr` CLI. Every installer path
# (this recipe + `ministr setup`) targets this one spot; everything else is a
# shadow to be removed.
CANONICAL_CLI="$HOME/.ministr/bin/ministr"

# Remove a stale `ministr` binary that would shadow the canonical one,
# escalating to sudo for root-owned copies (e.g. a /usr/local/bin left by an
# old installer). Loud on every outcome — the previous
# `rm -f … 2>/dev/null || true` silently swallowed root-owned failures, so a
# stale shadow could persist while the recipe claimed to have de-shadowed.
remove_cli_shadow() {
    local f="$1"
    [ -e "$f" ] || [ -L "$f" ] || return 0
    if rm -f "$f" 2>/dev/null; then
        echo "   removed stale CLI shadow: $f"
        return 0
    fi
    if command -v sudo >/dev/null 2>&1 && [ -t 1 ]; then
        echo "   '$f' is root-owned / not user-writable — removing with sudo (you may be prompted)…"
        if sudo rm -f "$f"; then
            echo "   removed with sudo: $f"
            return 0
        fi
    fi
    echo "   WARNING: could not remove CLI shadow: $f" >&2
    echo "            remove it manually so it can't shadow $CANONICAL_CLI:" >&2
    echo "              sudo rm -f '$f'" >&2
}

# After install + PATH wiring, confirm `ministr` actually resolves to the
# canonical binary and not a leftover shadow earlier on PATH.
verify_cli_canonical() {
    local resolved
    resolved="$(command -v ministr 2>/dev/null || true)"
    if [ -z "$resolved" ]; then
        echo "   note: \`ministr\` not on PATH yet — open a new shell or \`source ~/.ministr/env\`"
        return 0
    fi
    if [ "$resolved" = "$CANONICAL_CLI" ]; then
        echo "   verified: \`ministr\` -> $resolved"
    else
        echo "   WARNING: \`ministr\` resolves to a SHADOW: $resolved" >&2
        echo "            (expected $CANONICAL_CLI) — remove the shadow:" >&2
        echo "              sudo rm -f '$resolved'   # if root-owned" >&2
    fi
}

# ─── build ─────────────────────────────────────────────────────────────────────

echo "==> Clean rebuild (release)..."
cargo clean -p ministr-mcp -p ministr-cli -p ministr-daemon
cargo build --release -p ministr-cli

# ─── install CLI ──────────────────────────────────────────────────────────────

# Stop now (post-build, immediately before install) so nothing holds the
# binary open across the atomic rename.
stop_ministr

echo "==> Installing CLI to $CANONICAL_CLI (canonical dev location)..."
# Remove stale copies from other locations to prevent shadow binaries.
# Loud + sudo-escalating (see remove_cli_shadow) so a root-owned shadow such
# as a stale /usr/local/bin/ministr can never be silently left behind.
remove_cli_shadow "$HOME/.cargo/bin/ministr"
remove_cli_shadow "/usr/local/bin/ministr"
mkdir -p "$HOME/.ministr/bin"
# CLI isn't typically running as a long-lived daemon under this path, but
# use atomic_install anyway for parity — same cost, removes a foot-gun.
atomic_install target/release/ministr "$CANONICAL_CLI"

# Hand off PATH wiring to `ministr setup` (onpath crate). Detects
# installed shells and writes the right rc-file edits. Idempotent —
# re-runs of this dev recipe won't duplicate entries. Non-fatal: the
# binary is at ~/.ministr/bin/ministr regardless, so PATH-wiring trouble
# shouldn't abort the rest of the reinstall.
echo "==> Adding ministr to PATH via \`ministr setup\`..."
if ! "$CANONICAL_CLI" setup; then
    echo "   ministr setup failed — add manually with:" >&2
    echo "     export PATH=\"\$HOME/.ministr/bin:\$PATH\"" >&2
fi

# Confirm the CLI on PATH is the one we just installed, not a surviving shadow.
verify_cli_canonical

# ─── where everything landed ────────────────────────────────────────────────
echo
echo "==> Installed:"
echo "      CLI: $CANONICAL_CLI"
echo "==> Done. Restart your Claude Code session to pick up the new binary."
