# Installation

## From a release

```sh
curl -fsSL https://ministr.ai/install.sh | bash
```

On Windows:

```powershell
iwr -useb https://ministr.ai/install.ps1 | iex
```

Both install the newest stable release, falling back to the newest prerelease
while only prereleases exist. Set `MINISTR_VERSION` to pin an exact version,
and `INSTALL_DIR` to change where the binary lands.

## From source

```sh
cargo install --git https://github.com/OlsonSoftware/ministr --locked ministr-cli
```

Requires [Rust](https://rustup.rs) (rustup picks up the pinned toolchain
automatically) and a C toolchain. On Windows, add `--features directml` for
DirectML GPU acceleration.

From a clone:

```sh
cargo install --path ministr-cli --locked
```

## Supported platforms

| Platform | Prebuilt binary | Tested in CI |
|---|---|---|
| macOS, Apple Silicon | yes | yes |
| Linux x86_64 | yes | yes |
| Linux aarch64 | yes | no — built, not exercised |
| Windows x86_64 | yes | **no — built, not exercised** |
| macOS, Intel | no | no |
| Windows on ARM | no | no |

Windows binaries are published but no continuous-integration job builds or runs
the test suite on Windows, so treat that target as unproven and report anything
that breaks. Intel Macs have no artifact and will not get one — the ONNX
Runtime dependency dropped Intel-mac prebuilts. Build from source there.

Platform coverage is part of the [stability
policy](../reference/stability.md#unstable-surfaces).

## PATH setup

`ministr setup` adds the binary's directory to your PATH across shells. It is
idempotent; `--dry-run` previews the change and `--uninstall` reverses it.

## Where things live

| Location | What |
|---|---|
| `~/.ministr/` | data directory: per-project indexes, downloaded embedding models, logs |
| `~/.ministr/config.toml` | optional global configuration |
| `.ministr.toml` | per-project configuration, at your repo root |

## Next

[Quickstart](quickstart.md) — point ministr at a project and connect your
coding agent.
