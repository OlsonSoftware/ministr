#!/usr/bin/env bash
# ministr installer — downloads the latest release binary from GitHub.
# Usage: curl -fsSL https://raw.githubusercontent.com/AlrikOlson/ministr-rs/main/install.sh | bash
set -euo pipefail

REPO="AlrikOlson/ministr-rs"
INSTALL_DIR="${INSTALL_DIR:-$HOME/.ministr/bin}"

info() { printf '\033[1;34m%s\033[0m\n' "$*"; }
err()  { printf '\033[1;31merror:\033[0m %s\n' "$*" >&2; exit 1; }

# Detect OS
case "$(uname -s)" in
    Linux*)  os="unknown-linux-gnu" ;;
    Darwin*) os="apple-darwin" ;;
    *)       err "unsupported OS: $(uname -s)" ;;
esac

# Detect architecture
case "$(uname -m)" in
    x86_64|amd64)  arch="x86_64" ;;
    aarch64|arm64) arch="aarch64" ;;
    *)             err "unsupported architecture: $(uname -m)" ;;
esac

target="${arch}-${os}"
archive="ministr-${target}.tar.gz"

# Find latest release tag
info "Finding latest ministr release..."
tag=$(curl -fsSL "https://api.github.com/repos/${REPO}/releases/latest" \
    | grep '"tag_name"' | head -1 | cut -d'"' -f4)

[ -n "$tag" ] || err "could not determine latest release tag"
info "Latest release: ${tag}"

url="https://github.com/${REPO}/releases/download/${tag}/${archive}"

# Download and extract
info "Downloading ${archive}..."
tmpdir=$(mktemp -d)
trap 'rm -rf "$tmpdir"' EXIT

curl -fsSL "$url" -o "${tmpdir}/${archive}"
tar xzf "${tmpdir}/${archive}" -C "$tmpdir"

# Install
mkdir -p "$INSTALL_DIR"
mv "${tmpdir}/ministr" "${INSTALL_DIR}/ministr"
chmod +x "${INSTALL_DIR}/ministr"

info "Installed ministr to ${INSTALL_DIR}/ministr"

# PATH instructions
if ! echo "$PATH" | tr ':' '\n' | grep -qx "$INSTALL_DIR"; then
    echo ""
    info "Add ministr to your PATH:"
    echo "  export PATH=\"${INSTALL_DIR}:\$PATH\""
    echo ""
    echo "Add this line to your shell profile (~/.bashrc, ~/.zshrc, etc.)"
fi
