# ministr

Real codebase understanding for AI coding agents.

ministr is a local, MIT-licensed code intelligence MCP server. It gives AI
coding agents AST-level understanding of your codebase — semantic search across
code and docs, symbol navigation, real reference graphs, and cross-language
bridge detection across 40+ languages. It runs locally, embeds locally, and
works with any MCP client — Claude Code, Cursor, VS Code / Copilot.

## Install

```sh
cargo install --git https://github.com/OlsonSoftware/ministr --locked ministr-cli
```

Requires [Rust](https://rustup.rs) (rustup picks up the pinned toolchain
automatically) and a C toolchain. From a clone:
`cargo install --path ministr-cli --locked`. On Windows, add
`--features directml` for DirectML GPU acceleration.

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

Full documentation — tools, configuration, guides — at
[ministr.ai/docs](https://ministr.ai/docs).

[CHANGELOG](CHANGELOG.md) · [CONTRIBUTING](CONTRIBUTING.md) ·
[SECURITY](SECURITY.md) · [STEWARDSHIP](STEWARDSHIP.md)

## License

The local stack is [MIT](LICENSE) and builds the complete `ministr` binary with
no cloud or proprietary dependencies. ministr is open-core — see
[STEWARDSHIP.md](STEWARDSHIP.md) for the split and our commitments.
