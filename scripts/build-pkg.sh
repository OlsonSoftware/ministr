#!/usr/bin/env bash
# ┌─────────────────────────────────────────────────────────────────────────────┐
# │ build-pkg.sh — Build a signed, notarized macOS .pkg installer for ministr.    │
# │                                                                            │
# │ Produces a distribution package containing:                                │
# │   • ministr.app          → /Applications                                      │
# │   • ministr CLI binary   → /usr/local/bin/ministr                                │
# │   • PATH config       → /etc/paths.d/ministr                                  │
# │                                                                            │
# │ Requirements:                                                              │
# │   • Xcode Command Line Tools                                               │
# │   • "Developer ID Application" certificate in keychain                     │
# │   • "Developer ID Installer" certificate in keychain                       │
# │                                                                            │
# │ Environment variables:                                                     │
# │   APPLE_SIGNING_IDENTITY   — "Developer ID Application: Name (TEAMID)"    │
# │   APPLE_INSTALLER_IDENTITY — "Developer ID Installer: Name (TEAMID)"      │
# │   APPLE_ID                 — Apple account email (for notarization)        │
# │   APPLE_PASSWORD           — App-specific password (for notarization)      │
# │   APPLE_TEAM_ID            — 10-char team identifier                       │
# │   SKIP_NOTARIZE            — set to "1" to skip notarization (dev builds)  │
# └─────────────────────────────────────────────────────────────────────────────┘
set -euo pipefail

# ── Configuration ─────────────────────────────────────────────────────────────

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
APP_NAME="ministr"
BUNDLE_ID="ai.ministr.desktop"
TAURI_DIR="$REPO_ROOT/ministr-app/src-tauri"
INSTALLER_DIR="$REPO_ROOT/installer"
OUTPUT_DIR="$REPO_ROOT/target/pkg"

# Read version from src-tauri/Cargo.toml (single source of truth — the
# `version` field was removed from tauri.conf.json in c8881f3 so Tauri
# reads it from Cargo.toml directly).
VERSION=$(awk -F\" '/^version[[:space:]]*=[[:space:]]*"/ { print $2; exit }' "$TAURI_DIR/Cargo.toml")
if [[ -z "$VERSION" ]]; then
    echo "error: could not read version from $TAURI_DIR/Cargo.toml" >&2
    exit 1
fi

echo "═══════════════════════════════════════════════════════════════════"
echo "  ministr PKG builder — v${VERSION}"
echo "═══════════════════════════════════════════════════════════════════"

# ── Validate environment ──────────────────────────────────────────────────────

check_var() {
    if [[ -z "${!1:-}" ]]; then
        echo "error: $1 is not set" >&2
        echo "  $2" >&2
        exit 1
    fi
}

check_var APPLE_SIGNING_IDENTITY   'e.g. "Developer ID Application: Your Name (TEAMID)"'
check_var APPLE_INSTALLER_IDENTITY 'e.g. "Developer ID Installer: Your Name (TEAMID)"'

if [[ "${SKIP_NOTARIZE:-}" != "1" ]]; then
    check_var APPLE_ID       "Apple account email for notarization"
    check_var APPLE_PASSWORD "App-specific password for notarization"
    check_var APPLE_TEAM_ID  "10-character team identifier"
fi

# ── Detect target architecture ────────────────────────────────────────────────

ARCH=$(uname -m)
case "$ARCH" in
    arm64)  RUST_TARGET="aarch64-apple-darwin" ;;
    x86_64) RUST_TARGET="x86_64-apple-darwin" ;;
    *)      echo "error: unsupported architecture: $ARCH" >&2; exit 1 ;;
esac

echo ""
echo "  Target:  $RUST_TARGET"
echo "  Version: $VERSION"
echo "  Sign:    $APPLE_SIGNING_IDENTITY"
echo "  Install: $APPLE_INSTALLER_IDENTITY"
echo ""

# ── Clean staging area ────────────────────────────────────────────────────────

STAGING="$OUTPUT_DIR/staging"
rm -rf "$OUTPUT_DIR"
mkdir -p "$STAGING/components" "$STAGING/cli-root/usr/local/bin" "$STAGING/cli-root/etc/paths.d"

# ── Step 1: Build the Tauri app bundle ────────────────────────────────────────

echo "▸ Building ministr.app..."

export APPLE_SIGNING_IDENTITY
cd "$REPO_ROOT/ministr-app"

# When skipping notarization, fully unset the env vars that trigger
# Tauri's built-in notarization so it only signs.
if [[ "${SKIP_NOTARIZE:-}" == "1" ]]; then
    (
        unset APPLE_ID APPLE_PASSWORD APPLE_TEAM_ID APPLE_API_ISSUER APPLE_API_KEY APPLE_API_KEY_PATH
        pnpm run tauri build --bundles app --target "$RUST_TARGET"
    )
else
    pnpm run tauri build --bundles app --target "$RUST_TARGET"
fi

APP_BUNDLE="$REPO_ROOT/target/${RUST_TARGET}/release/bundle/macos/${APP_NAME}.app"
if [[ ! -d "$APP_BUNDLE" ]]; then
    echo "error: app bundle not found at $APP_BUNDLE" >&2
    exit 1
fi

echo "  ✓ App bundle: $APP_BUNDLE"

# ── Step 2: Build and sign the standalone CLI binary ──────────────────────────

echo "▸ Building ministr CLI..."

cd "$REPO_ROOT"
cargo build --release --package ministr-cli --target "$RUST_TARGET"

CLI_BINARY="$REPO_ROOT/target/${RUST_TARGET}/release/ministr-cli"
if [[ ! -f "$CLI_BINARY" ]]; then
    echo "error: CLI binary not found at $CLI_BINARY" >&2
    exit 1
fi

echo "▸ Signing CLI binary with hardened runtime..."

codesign --force --options runtime \
    --sign "$APPLE_SIGNING_IDENTITY" \
    --timestamp \
    --entitlements "$TAURI_DIR/Entitlements.plist" \
    "$CLI_BINARY"

echo "  ✓ CLI signed"

# ── Step 3: Prepare CLI payload ───────────────────────────────────────────────

cp "$CLI_BINARY" "$STAGING/cli-root/usr/local/bin/ministr"
chmod 755 "$STAGING/cli-root/usr/local/bin/ministr"

# /etc/paths.d/ministr ensures /usr/local/bin is on PATH for all shells.
# This is a no-op on most systems but guarantees it for edge cases.
echo "/usr/local/bin" > "$STAGING/cli-root/etc/paths.d/ministr"

# ── Step 4: Create component packages ────────────────────────────────────────

echo "▸ Creating component packages..."

# App component — non-relocatable so it always goes to /Applications
pkgbuild \
    --identifier "${BUNDLE_ID}.app" \
    --version "$VERSION" \
    --component "$APP_BUNDLE" \
    --install-location "/Applications" \
    --timestamp \
    --sign "$APPLE_INSTALLER_IDENTITY" \
    "$STAGING/components/ministr-app.pkg"

echo "  ✓ ministr-app.pkg"

# CLI component
pkgbuild \
    --identifier "${BUNDLE_ID}.cli" \
    --version "$VERSION" \
    --root "$STAGING/cli-root" \
    --install-location "/" \
    --timestamp \
    --sign "$APPLE_INSTALLER_IDENTITY" \
    "$STAGING/components/ministr-cli.pkg"

echo "  ✓ ministr-cli.pkg"

# ── Step 5: Stamp version into distribution.xml and HTML resources ────────────

echo "▸ Preparing installer resources..."

DIST_XML="$STAGING/distribution.xml"
RESOURCES="$STAGING/resources"
mkdir -p "$RESOURCES"

python3 -c "
import os, shutil, sys
version = sys.argv[1]
src_dir = sys.argv[2]
dist_src = sys.argv[3]
dist_dst = sys.argv[4]
res_dst = sys.argv[5]

# Stamp distribution.xml
with open(dist_src) as f:
    open(dist_dst, 'w').write(f.read().replace('__VERSION__', version))

# Copy resource files — stamp text files, copy binary files as-is
for name in os.listdir(src_dir):
    src = os.path.join(src_dir, name)
    dst = os.path.join(res_dst, name)
    if name.endswith(('.html', '.xml', '.txt', '.rtf')):
        with open(src) as f:
            open(dst, 'w').write(f.read().replace('__VERSION__', version))
    else:
        shutil.copy2(src, dst)
" "$VERSION" "$INSTALLER_DIR/resources" "$INSTALLER_DIR/distribution.xml" "$DIST_XML" "$RESOURCES"

# ── Step 6: Build the distribution package ────────────────────────────────────

echo "▸ Building distribution package..."

UNSIGNED_PKG="$STAGING/ministr-unsigned.pkg"
FINAL_PKG="$OUTPUT_DIR/ministr-${VERSION}.pkg"

productbuild \
    --distribution "$DIST_XML" \
    --resources "$RESOURCES" \
    --package-path "$STAGING/components" \
    --timestamp \
    --sign "$APPLE_INSTALLER_IDENTITY" \
    "$FINAL_PKG"

echo "  ✓ $FINAL_PKG"

# ── Step 7: Verify signature ─────────────────────────────────────────────────

echo "▸ Verifying package signature..."
pkgutil --check-signature "$FINAL_PKG"

# ── Step 8: Notarize and staple ───────────────────────────────────────────────

if [[ "${SKIP_NOTARIZE:-}" == "1" ]]; then
    echo ""
    echo "▸ Skipping notarization (SKIP_NOTARIZE=1)"
else
    echo ""
    echo "▸ Submitting for notarization (this may take a few minutes)..."

    xcrun notarytool submit "$FINAL_PKG" \
        --apple-id "$APPLE_ID" \
        --password "$APPLE_PASSWORD" \
        --team-id "$APPLE_TEAM_ID" \
        --wait

    echo "▸ Stapling notarization ticket..."
    xcrun stapler staple "$FINAL_PKG"

    echo "  ✓ Notarized and stapled"
fi

# ── Done ──────────────────────────────────────────────────────────────────────

SIZE=$(du -h "$FINAL_PKG" | cut -f1)
echo ""
echo "═══════════════════════════════════════════════════════════════════"
echo "  ✓ ministr-${VERSION}.pkg  ($SIZE)"
echo "    $FINAL_PKG"
echo "═══════════════════════════════════════════════════════════════════"
