# ministr

Real codebase understanding for AI coding agents.

ministr is a local, MIT-licensed code intelligence MCP server. It gives AI
coding agents AST-level understanding of your codebase — semantic search across
code and docs, symbol navigation, real reference graphs, and cross-language
bridge detection across 40+ languages. It runs locally, embeds locally, and
works with any MCP client — Claude Code, Cursor, VS Code / Copilot.

## Install

```sh
curl -fsSL https://ministr.ai/install.sh | bash
```

Or build it yourself:

```sh
cargo install --git https://github.com/OlsonSoftware/ministr --locked ministr-cli
```

Building requires [Rust](https://rustup.rs) (rustup picks up the pinned
toolchain automatically) and a C toolchain. From a clone:
`cargo install --path ministr-cli --locked`. On Windows, add
`--features directml` for DirectML GPU acceleration.

Prebuilt binaries cover macOS on Apple Silicon, Linux x86_64 and aarch64, and
Windows x86_64. The Windows build is published but not exercised by CI, and
Intel Macs need a source build — details in
[installation](docs/getting-started/installation.md#supported-platforms).

## Use

```sh
cd your-project
ministr init
```

`ministr init` writes `.ministr.toml` — corpus paths auto-detected from your
project manifests — and the MCP configs for Claude Code, Cursor, and
VS Code / Copilot. Indexing happens automatically the first time an agent
connects.

## Learn more

Documentation — installation, client setup, configuration, tool reference —
lives in [docs/](docs/README.md). Configuration examples in
[examples/](examples/README.md); agent-facing usage notes in
[AGENTS.md](AGENTS.md).

[Stability](docs/reference/stability.md) states which surfaces the 1.x line
promises not to break. `ministr ui`, the terminal console, is a preview and is
deliberately excluded from that promise.

[CHANGELOG](CHANGELOG.md) · [CONTRIBUTING](CONTRIBUTING.md) ·
[SECURITY](SECURITY.md) · [STEWARDSHIP](STEWARDSHIP.md)

## License

The local stack is [MIT](LICENSE) and builds the complete `ministr` binary with
no cloud or proprietary dependencies. ministr is open-core — see
[STEWARDSHIP.md](STEWARDSHIP.md) for the split and our commitments.
