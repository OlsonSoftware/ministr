#!/usr/bin/env bash
# ministr installer — downloads the latest release binary from GitHub.
# Usage: curl -fsSL https://ministr.app/install.sh | bash
set -euo pipefail

GITHUB_REPO="${MINISTR_GITHUB_REPO:-OlsonSoftware/ministr}"
DL_HOST="${MINISTR_DL_HOST:-https://github.com/${GITHUB_REPO}/releases/download}"
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

# Find latest release tag via the GitHub API.
info "Finding latest ministr release..."
tag=$(curl -fsSL "https://api.github.com/repos/${GITHUB_REPO}/releases/latest" \
    | grep '"tag_name"' | head -1 | cut -d'"' -f4)

[ -n "$tag" ] || err "could not determine latest release tag"
info "Latest release: ${tag}"

url="${DL_HOST}/${tag}/${archive}"

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

# Hand off PATH wiring to `ministr setup`, which uses the onpath crate to
# detect installed shells (bash, zsh, fish, nushell, PowerShell, tcsh,
# xonsh) and write the right rc-file edits. Idempotent — re-running won't
# duplicate entries. Falls back to printing manual export instructions if
# the subcommand exits non-zero (e.g. no detected shell rc files).
if ! "${INSTALL_DIR}/ministr" setup --bin-dir "${INSTALL_DIR}"; then
    echo ""
    info "Could not auto-configure PATH — add this to your shell profile:"
    echo "  export PATH=\"${INSTALL_DIR}:\$PATH\""
fi
