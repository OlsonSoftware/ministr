# Installation

## CLI

### Homebrew (macOS)

```sh
brew install AlrikOlson/tap/iris
```

### Install script (macOS & Linux)

```sh
curl -fsSL https://raw.githubusercontent.com/AlrikOlson/iris-rs/main/install.sh | bash
```

### Cargo (from source, requires Rust 1.85+)

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

A macOS `.pkg` installer is available that includes both the desktop app (system tray) and CLI:

```sh
just pkg       # signed + notarized build
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
