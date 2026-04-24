<p align="center">
  <h1 align="center">ministr</h1>
</p>

<p align="center">
  <strong>Context cache for LLM agents</strong>
</p>

<p align="center">
  <a href="https://github.com/AlrikOlson/ministr-rs/actions/workflows/ci.yml"><img src="https://github.com/AlrikOlson/ministr-rs/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
  <a href="LICENSE-MIT"><img src="https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg" alt="License"></a>
  <a href="https://www.rust-lang.org"><img src="https://img.shields.io/badge/rust-1.88%2B-orange.svg" alt="Rust"></a>
</p>

<p align="center">
  <a href="https://ministr.ai">Docs</a> · <a href="CONTRIBUTING.md">Contributing</a> · <a href="CHANGELOG.md">Changelog</a>
</p>

---

ministr is an [MCP server](https://modelcontextprotocol.io) that serves your agent's context the way an L1 cache serves the CPU — tracking what it has delivered, pre-warming what's likely next, and flagging budget pressure. Your agent still owns its context window; ministr just keeps its own output lean. Runs locally, embeds locally, works with any MCP client.

```sh
claude mcp add ministr -- ministr
```

<p align="center">
  <video
    src="https://github.com/OlsonSoftware/ministr/raw/main/assets/launch.mp4"
    poster="https://github.com/OlsonSoftware/ministr/raw/main/assets/launch.gif"
    alt="ministr CLI demo — ministr init, claude mcp add ministr, and an agent trace with a cache hit"
    width="860"
    controls
    muted
    preload="metadata">
  </video>
</p>

<p align="center">
  <sub>Prefer an interactive version with selectable text and timeline scrubbing? <a href="https://ministr.ai/#live-demo">Watch it on the docs site</a>.</sub>
</p>

## Why ministr

LLM agents waste most of their context window. ministr fixes the three root causes:

**Re-reading** — ministr tracks what the agent has already seen and deduplicates. When a section changes, it delivers only the delta.

**Blind retrieval** — ministr indexes your codebase at multiple resolutions (documents, sections, claims, symbols) and returns precisely what's relevant — not entire files.

**No lookahead** — ministr predicts what the agent will need next and pre-warms it. Sequential, structural, and topical prefetch strategies mean cache hits instead of cold reads.

## Setup

**1.** Create `.ministr.toml` in your project root (or run `ministr init`):

```toml
[corpus]
paths = ["src", "docs", "README.md"]
```

**2.** Connect your MCP client:

```sh
claude mcp add ministr -- ministr                                    # Claude Code
```

```json
{ "mcpServers": { "ministr": { "command": "ministr", "args": [] } } }
```

<sup>Save as <code>.mcp.json</code> (Claude Code) or <code>.cursor/mcp.json</code> (Cursor). ministr auto-discovers <code>.ministr.toml</code> from the working directory.</sup>

## Features

- **Semantic search** across docs and code at document, section, and claim resolution
- **Code symbol navigation** — find and trace structs, functions, traits across 12 languages via tree-sitter
- **Cross-language bridge detection** — Tauri commands, napi bindings, PyO3 functions, wasm-bindgen exports, HTTP routes
- **Session tracking** with predictive prefetch, deduplication, and delta delivery
- **Budget management** — token usage monitoring, eviction recommendations, compressed summaries under pressure
- **Local embeddings** — FastEmbed + ONNX (~5ms/embed), optional Metal GPU acceleration on Apple Silicon
- **Desktop app** — cache observatory dashboard, live tool-call activity stream, `⌘K` command palette, `?` shortcut sheet, and system-tray submenus (Tauri v2, macOS/Linux/Windows)

## Cross-language bridges

ministr detects and links cross-language bindings automatically:

<p align="center">
  <img src="assets/bridges.svg" alt="Cross-language bridge diagram: Rust exports (napi, PyO3, Tauri) linked to JavaScript/Python consumers" width="720">
</p>

## Installation

**Install script** (macOS & Linux)

```sh
curl -fsSL https://ministr.app/install.sh | bash
```

**Cargo** (latest `main`)

```sh
cargo install --git https://github.com/AlrikOlson/ministr-rs ministr-cli
```

**Pre-built binaries** — download from [GitHub Releases](https://github.com/AlrikOlson/ministr-rs/releases) for macOS, Linux, and Windows.

A Homebrew tap (`AlrikOlson/homebrew-tap`) and a crates.io publish land with 1.0.

## Documentation

| | |
|---|---|
| [Docs home](https://ministr.ai/) | Landing page with full overview |
| [Tool reference](https://ministr.ai/docs/tools/) | All MCP tools with parameters and examples |
| [Architecture](https://ministr.ai/docs/architecture-deep-dive/) | Crate structure, layering, and subsystem deep dive |
| [Configuration](https://ministr.ai/docs/configuration/) | `.ministr.toml` options and CLI flags |
| [Design specification](DESIGN.md) | Research references and design rationale |
| [Deployment](deploy/README.md) | Docker, Fly.io, Railway, nginx/Caddy reverse proxy |
| [Example configs](examples/) | `.ministr.toml` templates for Rust, Tauri, PyO3, React |

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md) for development setup and guidelines.

## License

MIT OR Apache-2.0. See [LICENSE-MIT](LICENSE-MIT) and [LICENSE-APACHE](LICENSE-APACHE).
