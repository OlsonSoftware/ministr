# Installation

!!! tip "Want the desktop app too?"

    The [**Download page**](download.md) has the signed `.pkg` that installs
    both `iris.app` **and** the `iris` CLI in one go (macOS only). This
    page covers the CLI-only and dev-build paths.

## CLI

### Install script (macOS & Linux)

```sh
curl -fsSL https://raw.githubusercontent.com/AlrikOlson/iris-rs/main/install.sh | bash
```

The script detects your platform, downloads the matching archive from
GitHub Releases, verifies the SHA-256 checksum, and installs `iris` to
`/usr/local/bin` (or `$HOME/.local/bin` without sudo).

### Cargo (from source, requires Rust 1.85+)

Install the latest `main` directly from the repository:

```sh
cargo install --git https://github.com/AlrikOlson/iris-rs iris-cli
```

### Homebrew (macOS) — coming with 1.0

A formula lives in the `homebrew/` directory of this repo. Once the
`AlrikOlson/homebrew-tap` repo is published, install with:

```sh
brew install AlrikOlson/tap/iris
```

### crates.io — coming with 1.0

When the workspace is published to crates.io, install with:

```sh
cargo install iris-cli
```

### Pre-built binaries

Download from [GitHub Releases](https://github.com/AlrikOlson/iris-rs/releases) — builds available for:

| Platform | Archive |
|----------|---------|
| macOS (Apple Silicon) | `iris-aarch64-apple-darwin.tar.gz` |
| macOS (Intel) | `iris-x86_64-apple-darwin.tar.gz` |
| Linux (x86_64) | `iris-x86_64-unknown-linux-gnu.tar.gz` |
| Linux (aarch64) | `iris-aarch64-unknown-linux-gnu.tar.gz` |
| Windows (x86_64) | `iris-x86_64-pc-windows-msvc.zip` |

Each archive has a corresponding `.sha256` checksum file.

## Desktop App (macOS)

End users should grab the pre-built `.pkg` from the [Download page](download.md) —
it's signed, notarized, and installs both `iris.app` and the `iris` CLI with
`PATH` wiring done for you.

Building locally from source (for maintainers):

```sh
just pkg       # signed + notarized build (requires Apple Developer ID)
just pkg-dev   # local testing (no notarization)
```

See `installer/SIGNING-GUIDE.md` for code signing setup.

## Configuration

Per-project config: `.iris.toml` in the project root (created by `iris init`):

```toml
[corpus]
paths = ["src", "docs", "README.md"]
ignore = ["*.snap", "node_modules"]
```

See [Configuration](configuration.md) for full details.
