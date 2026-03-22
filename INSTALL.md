# Installing iris

## Pre-built binaries (recommended)

Download the latest release from [GitHub Releases](https://github.com/alrik/iris-rs/releases/latest).

| Platform | Archive |
|----------|---------|
| Linux x86_64 | `iris-x86_64-unknown-linux-gnu.tar.gz` |
| Linux aarch64 | `iris-aarch64-unknown-linux-gnu.tar.gz` |
| macOS Apple Silicon | `iris-aarch64-apple-darwin.tar.gz` |
| Windows x86_64 | `iris-x86_64-pc-windows-msvc.zip` |

Each archive has a corresponding `.sha256` checksum file.

### Verify and install (Unix)

```sh
# Download the archive and checksum for your platform
curl -LO https://github.com/alrik/iris-rs/releases/latest/download/iris-aarch64-apple-darwin.tar.gz
curl -LO https://github.com/alrik/iris-rs/releases/latest/download/iris-aarch64-apple-darwin.tar.gz.sha256

# Verify checksum
shasum -a 256 -c iris-aarch64-apple-darwin.tar.gz.sha256

# Extract and install
tar xzf iris-aarch64-apple-darwin.tar.gz
install -m 755 iris /usr/local/bin/
```

## From source (via cargo)

Requires Rust 1.85+ toolchain.

```sh
cargo install --git https://github.com/alrik/iris-rs iris-cli
```

Once published to crates.io:

```sh
cargo install iris-cli
```

## Homebrew (planned)

A Homebrew tap will be available after the first release:

```sh
brew install alrik/tap/iris
```
