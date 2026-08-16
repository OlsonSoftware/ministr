#!/usr/bin/env bash
# ministr installer — downloads a release binary from GitHub.
#
# Usage:
#   curl -fsSL https://ministr.ai/install.sh | bash
#
# Environment overrides:
#   MINISTR_VERSION      install this version instead of the newest
#                        (e.g. 1.0.0-beta.1; a leading `v` is optional)
#   MINISTR_GITHUB_REPO  override the repo (default: OlsonSoftware/ministr)
#   MINISTR_DL_HOST      override the download host (testing / mirrors)
#   INSTALL_DIR          override the install location (default ~/.ministr/bin)
#
# Supported: macOS (Apple Silicon), Linux (x86_64, aarch64).
# Intel Mac and Windows-on-ARM are not shipped — build from source.
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

# Intel Macs have no release artifact and never will: ort-sys dropped
# Intel-mac prebuilts and macOS 26 dropped Intel. Say so, rather than
# 404ing on an asset name we deliberately don't build.
if [ "$os" = "apple-darwin" ] && [ "$arch" = "x86_64" ]; then
    err "Intel Macs are not supported (Apple Silicon only).
  Build from source instead:
    cargo install --git https://github.com/${GITHUB_REPO} --locked ministr-cli"
fi

target="${arch}-${os}"
archive="ministr-${target}.tar.gz"

api="https://api.github.com/repos/${GITHUB_REPO}"

# Pull the first "tag_name" out of a GitHub API response.
parse_tag() { sed -n 's/.*"tag_name"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' | head -1; }

# Resolve which tag to install.
#
#   MINISTR_VERSION=1.0.0-beta.1  → pin exactly (leading `v` optional)
#   otherwise                     → newest stable, else newest prerelease
#
# /releases/latest EXCLUDES prereleases and 404s on a repo that has only
# prereleases — which is exactly the state of a beta line. Falling back
# to /releases (newest first, all kinds) is what makes `curl | bash`
# work during a beta.
resolve_tag() {
    if [ -n "${MINISTR_VERSION:-}" ]; then
        case "$MINISTR_VERSION" in
            v*) printf '%s' "$MINISTR_VERSION" ;;
            *)  printf 'v%s' "$MINISTR_VERSION" ;;
        esac
        return 0
    fi

    local t
    t="$(curl -fsSL "${api}/releases/latest" 2>/dev/null | parse_tag || true)"
    if [ -z "$t" ]; then
        t="$(curl -fsSL "${api}/releases?per_page=1" 2>/dev/null | parse_tag || true)"
        [ -n "$t" ] && info "No stable release yet — installing the newest prerelease."
    fi
    printf '%s' "$t"
}

info "Finding latest ministr release..."
tag="$(resolve_tag)"

if [ -z "$tag" ]; then
    err "no published releases found for ${GITHUB_REPO}.
  Install from source instead:
    cargo install --git https://github.com/${GITHUB_REPO} --locked ministr-cli
  Or pin a specific version once one is published:
    MINISTR_VERSION=1.0.0-beta.1 curl -fsSL https://ministr.ai/install.sh | bash"
fi
info "Installing: ${tag}"

url="${DL_HOST}/${tag}/${archive}"

# Download and extract
info "Downloading ${archive}..."
tmpdir=$(mktemp -d)
trap 'rm -rf "$tmpdir"' EXIT

if ! curl -fsSL "$url" -o "${tmpdir}/${archive}"; then
    err "no ${archive} attached to release ${tag}.
  ${target} may not be built for this version. Build from source:
    cargo install --git https://github.com/${GITHUB_REPO} --locked ministr-cli"
fi
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
