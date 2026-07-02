# Installation

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
