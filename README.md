<p align="center">
  <h1 align="center">ministr</h1>
</p>

<p align="center">
  <strong>Codebase intelligence for AI agents</strong>
</p>

<p align="center">
  <a href="https://www.rust-lang.org"><img src="https://img.shields.io/badge/rust-1.88%2B-orange.svg" alt="Rust"></a>
</p>

<p align="center">
  <a href="https://ministr.ai">Docs</a> · <a href="https://ministr.ai/install">Install</a> · <a href="CHANGELOG.md">Changelog</a>
</p>

---

ministr is an [MCP server](https://modelcontextprotocol.io) that gives Claude Code, Cursor, and Copilot real codebase intelligence — semantic search across your code and docs, symbol-level navigation, and cross-language bridge detection — and it remembers what it has already shown the agent so the same context isn't re-fetched on every turn. Runs locally, embeds locally, works with any MCP client.

```sh
claude mcp add ministr -- ministr
```

<p align="center">
  <a href="https://ministr.ai/"><img src="assets/launch.gif" alt="ministr demo — ministr init, claude mcp add ministr, and an agent trace with semantic search results" width="860"></a>
</p>

<p align="center">
  <sub>Watch the interactive version with selectable text and timeline scrubbing on <a href="https://ministr.ai/">ministr.ai</a>.</sub>
</p>

## Why ministr

AI coding agents waste a lot of their context window. ministr fixes three of those wastes:

**Re-reading** — ministr tracks what the agent has already seen and deduplicates. When a section changes, it delivers only the delta.

**Blind retrieval** — ministr indexes your codebase at multiple resolutions (documents, sections, claims, symbols) and returns precisely what's relevant — not entire files.

**No lookahead** — ministr predicts what the agent will need next and pre-warms it. Six prefetch strategies across reads (sequential, structural, topical, cross-session) and surveys (expand + intent) mean the next tool call is already warm instead of cold.

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

- **Semantic search** across docs and code at the granularity the agent needs — summary, section, or single claim (a one-sentence fact pulled from a section). Code gets two extra levels: symbol stub (signature + doc) and full source.
- **Code symbol navigation** — find and trace structs, functions, traits across ~29 languages via tree-sitter (Rust, Python, JS/TS, Go, Java, C/C++, C#, Ruby, Swift, Kotlin, PHP, Scala, Bash, Lua, Elixir, Haskell, OCaml, Dart, R, HCL/Terraform, SQL, Zig, Protobuf, Svelte, JSON/YAML/TOML)
- **Cross-language bridge detection** — Tauri commands and events, napi-rs, PyO3, wasm-bindgen, HTTP routes (actix-web / axum / rocket), cgo (Go ↔ C), JNI, UniFFI, gRPC, and raw FFI
- **Session tracking** with predictive prefetch, deduplication, and delta delivery
- **Budget management** — token usage monitoring, eviction recommendations, compressed summaries under pressure
- **Local embeddings** — Candle with Metal GPU on Apple Silicon by default (7-12× faster than the ONNX path for batch embedding); FastEmbed + DirectML on Windows DirectX 12 GPUs (with the `directml` cargo feature); FastEmbed + CPU ONNX on Linux and feature-less Windows
- **Desktop app** — dashboard, live tool-call activity stream, `⌘K` command palette, `?` shortcut sheet, and system-tray submenus (Tauri v2, macOS/Linux/Windows)

## Cross-language bridges

ministr detects and links cross-language bindings automatically:

<p align="center">
  <img src="assets/bridges.svg" alt="Cross-language bridge diagram: Rust exports (napi, PyO3, Tauri) linked to JavaScript/Python consumers" width="720">
</p>

## Installation

→ **[ministr.ai/install](https://ministr.ai/install)** — desktop installers for macOS / Windows / Linux, plus a one-line CLI install for any platform.

Quick install (macOS & Linux):

```sh
curl -fsSL https://ministr.app/install.sh | bash
```

## Documentation

| | |
|---|---|
| [Docs home](https://ministr.ai/) | Landing page with full overview |
| [Install](https://ministr.ai/install) | Desktop installers and CLI install scripts |
| [Tool reference](https://ministr.ai/docs/tools/) | All MCP tools with parameters and examples |
| [Architecture](https://ministr.ai/docs/architecture-deep-dive/) | System architecture and subsystem deep dive |
| [Configuration](https://ministr.ai/docs/configuration/) | `.ministr.toml` options and CLI flags |
