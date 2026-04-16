# iris

[![CI](https://github.com/alrik/iris-rs/actions/workflows/ci.yml/badge.svg)](https://github.com/alrik/iris-rs/actions/workflows/ci.yml)
[![Crates.io](https://img.shields.io/crates/v/iris-cli.svg)](https://crates.io/crates/iris-cli)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](LICENSE-MIT)

**A context cache controller for LLM agents.**

iris is an [MCP server](https://modelcontextprotocol.io) that manages your agent's context window like a CPU cache controller — with session tracking, predictive prefetching, budget management, and coherence. It runs locally, embeds locally (no API keys), and works with any MCP client.

```
┌─ Agent ──────────────────────────────────────────────────────────────┐
│                                                                      │
│  "What does the auth middleware do?"                                 │
│                                                                      │
└──────────────────────────┬───────────────────────────────────────────┘
                           │ MCP
┌─ iris ───────────────────▼───────────────────────────────────────────┐
│                                                                      │
│  Session Shadow    Prefetch Engine    Budget Manager    Coherence    │
│  ┌─────────────┐  ┌──────────────┐  ┌─────────────┐  ┌──────────┐  │
│  │ tracks what │  │ pre-warms    │  │ estimates   │  │ watches  │  │
│  │ the agent   │  │ content the  │  │ token usage │  │ files &  │  │
│  │ has seen    │  │ agent will   │  │ & recommends│  │ alerts   │  │
│  │ & evicted   │  │ need next    │  │ evictions   │  │ on stale │  │
│  └─────────────┘  └──────────────┘  └─────────────┘  └──────────┘  │
│                                                                      │
│  HNSW Vector Index  ←  FastEmbed (local, ~5ms/embed)                │
│  SQLite Storage     ←  Session persistence across restarts          │
│  Tree-sitter AST    ←  12 languages, symbol index, cross-lang links │
│                                                                      │
└──────────────────────────────────────────────────────────────────────┘
```

## Cross-language bridge detection

iris detects and links cross-language bindings automatically — napi, pyo3, tauri, wasm-bindgen, and HTTP route matching:

```
┌─ Rust ──────────────────┐         ┌─ JavaScript ──────────────┐
│                         │         │                           │
│ #[napi]                 │ ══napi══│ import { greet }          │
│ fn greet(s: String)     │         │ from './native'           │
│                         │         │                           │
│ #[pyfunction]           │ ══pyo3══│ from mylib import         │
│ fn compute(x: f64)      │         │     compute               │
│                         │         │                           │
│ #[tauri::command]       │ ═tauri══│ invoke('open_file',       │
│ fn open_file(path)      │         │    { path })              │
└─────────────────────────┘         └───────────────────────────┘
```

Use `iris_bridge` to query these links, or `iris_references` to trace a symbol across language boundaries.

## Features

- **Semantic search** — query documents and code with embedding-based retrieval at multiple resolutions (document, section, claim)
- **Code symbol index** — find structs, functions, traits, enums across 12 languages by name, kind, module, or visibility
- **Cross-language bridges** — automatic detection of napi, pyo3, tauri, wasm-bindgen, and HTTP route bindings
- **Session tracking** — shadow the agent's context window to deduplicate delivered content and track evictions
- **Predictive prefetch** — speculatively pre-embed content the agent is likely to request next (sequential, structural, topical, cross-session)
- **Budget management** — monitor token usage, recommend evictions, and provide compressed summaries under pressure
- **Live coherence** — watch the filesystem for changes and alert the agent when delivered content goes stale
- **Delta delivery** — only send changed lines when re-reading a modified section
- **Multi-source corpora** — index local directories, web URLs, and git repositories
- **Local embeddings** — FastEmbed with ONNX runtime (~5ms/embed), CoreML acceleration on Apple Silicon, no API keys
- **Single instance per repo** — automatic stdio-to-HTTP proxy when a second client connects

## Installation

### Homebrew (macOS)

```sh
brew install alrik/tap/iris
```

### Install script (macOS & Linux)

```sh
curl -fsSL https://raw.githubusercontent.com/alrik/iris-rs/main/install.sh | bash
```

### Cargo

```sh
cargo install iris-cli
```

### Pre-built binaries

Download from [GitHub Releases](https://github.com/alrik/iris-rs/releases) — builds available for:
- macOS (Apple Silicon & Intel)
- Linux (x86_64 & aarch64)
- Windows (x86_64)

## Quick start

### 1. Create `.iris.toml` in your project root

```toml
[corpus]
paths = [
    "src",           # Source code
    "docs",          # Documentation
    "README.md",     # Project overview
]

ignore = [
    "*.snap",        # Test snapshots
    "node_modules",  # Dependencies
]
```

### 2. Configure your MCP client

**Claude Code:**

```sh
claude mcp add iris -- iris
```

Or create `.mcp.json` at your project root:

```json
{
  "mcpServers": {
    "iris": {
      "command": "iris",
      "args": []
    }
  }
}
```

**Cursor** — create `.cursor/mcp.json`:

```json
{
  "mcpServers": {
    "iris": {
      "command": "iris",
      "args": []
    }
  }
}
```

iris auto-discovers `.iris.toml` by walking up from the working directory. No `--corpus` flag needed when using a config file.

### 3. (Optional) Pre-warm the index

```sh
iris index
```

This is optional — iris indexes on first MCP connection. Pre-warming is useful for large corpora to avoid the initial indexing delay.

### 4. Start using iris tools

Once connected, the agent has access to these MCP tools:

| Tool | Purpose |
|------|---------|
| `iris_survey` | Semantic search across docs and code |
| `iris_read` | Read a section by ID (with deduplication & delta delivery) |
| `iris_extract` | Get atomic claims from a section |
| `iris_related` | Follow dependency chains between claims |
| `iris_symbols` | Search the code symbol index |
| `iris_definition` | Get full source of a symbol |
| `iris_references` | Find callers, implementors, importers |
| `iris_bridge` | Query cross-language bindings |
| `iris_budget` | Check context budget and get eviction advice |
| `iris_compress` | Generate compressed summaries for eviction |
| `iris_evicted` | Signal that content has been dropped |
| `iris_fetch` | Fetch web content and add to corpus |
| `iris_clone` | Clone a git repo and index it |
| `iris_refresh` | Re-fetch changed web sources |
| `iris_toc` | Structural overview of the corpus |

## How it works

iris treats the context window as L1 cache, not memory. It tracks what the agent has seen, predicts what it will need next, and manages evictions when the budget runs low:

```
 Time   Agent Action                iris Response                Internal State
─────┬──────────────────────┬──────────────────────────────┬─────────────────────
  1  │ iris_survey(         │ Top 5 ranked results         │ Prefetch: pre-warm
     │   "auth middleware") │ budget: 3% used              │ siblings of top hit
     │                      │                              │
  2  │ iris_read(           │ Full section text             │ Prefetch: pre-warm
     │   "src/auth.rs#      │ budget: 5.5% used            │ logout (sequential)
     │    login")           │                              │ validate (structural)
     │                      │                              │
  3  │ iris_read(           │ CACHE HIT! Instant delivery  │ Sequential prefetch
     │   "src/auth.rs#      │ (was pre-warmed)             │ paid off!
     │    logout")          │ budget: 7% used              │
     │                      │                              │
 ... │  (agent works)       │                              │ budget: 82% used
     │                      │                              │
  N  │ iris_survey(         │ Results at CLAIM resolution  │ Pressure: ELEVATED
     │   "error handling")  │ + eviction_recommendations   │ Compressed responses
     │                      │                              │
 N+1 │ iris_evicted(        │ Budget freed                 │ Session shadow
     │   ["old-section"])   │ budget: 75% used             │ updated
```

## Configuration

### `.iris.toml`

```toml
[corpus]
paths = ["src", "docs", "README.md"]
ignore = ["*.snap", "target", "node_modules"]
```

### CLI

```
iris [OPTIONS] [COMMAND]

Commands:
  serve   Start the MCP server over stdio (default)
  index   Run ingestion synchronously and exit

Options:
  -c, --corpus <PATH>   Corpus sources (repeatable): local paths, https:// URLs, github:// URLs
  -C, --config <PATH>   Path to config file (default: .iris.toml or ~/.iris/config.toml)
  -h, --help            Print help
  -V, --version         Print version
```

## Architecture

```
iris-core/          — domain logic, no transport dependencies
iris-api/           — shared request/response types for daemon ↔ MCP/CLI
iris-daemon/        — HTTP API over Unix domain socket
iris-mcp/           — MCP server adapter (rmcp)
iris-cli/           — binary entry point
iris-app/src-tauri/ — Tauri v2 desktop app with system tray
```

See [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) for the deep dive.

## Supported languages

Tree-sitter parsing and symbol extraction for: **Rust**, **Python**, **JavaScript**, **TypeScript**, **Go**, **Java**, **C**, **C++**, **Ruby**, **C#**, **Swift**, **Kotlin**.

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md) for development setup, architecture overview, and PR guidelines.

## License

Licensed under either of

- [Apache License, Version 2.0](LICENSE-APACHE)
- [MIT License](LICENSE-MIT)

at your option.

## Contribution

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in the work by you, as defined in the Apache-2.0 license, shall be dual licensed as above, without any additional terms or conditions.
