# iris

[![CI](https://github.com/alrik/iris-rs/actions/workflows/ci.yml/badge.svg)](https://github.com/alrik/iris-rs/actions/workflows/ci.yml)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](LICENSE-MIT)
[![Rust: 1.85+](https://img.shields.io/badge/rust-1.85%2B-orange.svg)](https://www.rust-lang.org)

**Context cache controller for LLM agents.**

iris is an [MCP server](https://modelcontextprotocol.io) that manages your agent's context window like L1 cache — tracking what it has seen, predicting what it needs next, and managing evictions when budget runs low. Local embeddings, 12-language code navigation, cross-language bridge detection. No API keys required.

```sh
claude mcp add iris -- iris    # that's it
```

## How it works

iris treats the context window as cache, not memory. Every tool response includes budget tracking, and the prefetch engine speculatively pre-warms content before the agent asks for it:

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
  3  │ iris_read(           │ CACHE HIT — instant delivery │ Sequential prefetch
     │   "src/auth.rs#      │ (was pre-warmed at step 2)   │ paid off
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

## Quick start

**1. Initialize**

```sh
cd your-project
iris init        # creates .iris.toml + .mcp.json
```

Or write `.iris.toml` yourself:

```toml
[corpus]
paths = ["src", "docs", "README.md"]
ignore = ["*.snap", "node_modules"]
```

**2. Connect your MCP client**

```sh
claude mcp add iris -- iris              # Claude Code
```
```json
{"mcpServers": {"iris": {"command": "iris", "args": []}}}
```
<sup>`.cursor/mcp.json` for Cursor, `.mcp.json` for Claude Code</sup>

**3. Use it**

The agent now has semantic search, code navigation, and budget management. iris indexes on first connection — no manual step needed.

## Features

### Search & retrieval

- **Semantic search** — embedding-based retrieval at document, section, and claim resolution
- **Code symbol index** — structs, functions, traits, enums across 12 languages via tree-sitter
- **Cross-language bridges** — automatic detection of Tauri, napi, PyO3, wasm-bindgen, and HTTP route bindings
- **Multi-source corpora** — index local directories, web URLs, and git repositories

### Session intelligence

- **Session shadow** — tracks what the agent has seen, deduplicates deliveries, detects evictions
- **Predictive prefetch** — pre-warms content using sequential, structural, topical, and cross-session locality
- **Budget management** — monitors token usage, recommends evictions, provides compressed summaries under pressure
- **Delta delivery** — only sends changed lines when re-reading a modified section

### Infrastructure

- **Local embeddings** — FastEmbed + ONNX (~5ms/embed), optional Metal GPU via Candle on Apple Silicon
- **Live coherence** — watches the filesystem, re-indexes on change, alerts the agent about stale content
- **Single instance** — automatic stdio-to-HTTP proxy when a second client connects to a running daemon
- **Streamable HTTP** — remote deployment via Docker, Fly.io, or Railway

## Tools

| Tool | What it does |
|------|-------------|
| `iris_survey` | Semantic search across docs and code |
| `iris_symbols` | Find symbols by name, kind, or module |
| `iris_definition` | Full source of a symbol |
| `iris_references` | Callers, implementors, importers |
| `iris_read` | Read a section (with dedup and delta delivery) |
| `iris_extract` | Atomic claims from a section |
| `iris_bridge` | Cross-language binding links |
| `iris_budget` | Context budget status and eviction advice |

<details>
<summary>All tools</summary>

| Tool | What it does |
|------|-------------|
| `iris_compress` | Compressed summaries for eviction |
| `iris_evicted` | Signal that content was dropped |
| `iris_related` | Follow dependency chains between claims |
| `iris_toc` | Structural overview of the corpus |
| `iris_fetch` | Fetch web content into the corpus |
| `iris_clone` | Clone and index a git repo |
| `iris_refresh` | Re-fetch changed web sources |

</details>

## Installation

**Homebrew** (macOS)
```sh
brew install alrik/tap/iris
```

**Install script** (macOS & Linux)
```sh
curl -fsSL https://raw.githubusercontent.com/alrik/iris-rs/main/install.sh | bash
```

**Cargo** (from source)
```sh
cargo install iris-cli
```

**Pre-built binaries** — [GitHub Releases](https://github.com/alrik/iris-rs/releases) for macOS (Apple Silicon & Intel), Linux (x86_64 & aarch64), Windows (x86_64).

## Cross-language bridges

iris detects and links cross-language bindings automatically:

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

## Supported languages

Tree-sitter parsing and symbol extraction for **Rust**, **Python**, **JavaScript**, **TypeScript**, **Go**, **Java**, **C**, **C++**, **Ruby**, **C#**, **Swift**, **Kotlin**.

## Documentation

- [Architecture](docs/ARCHITECTURE.md) — deep dive into the crate structure and subsystems
- [Design](DESIGN.md) — the full design specification and research references
- [mdBook docs](https://alrik.github.io/iris-rs) — user guide, tool reference, and configuration

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md) for development setup and PR guidelines.

## License

MIT OR Apache-2.0, at your option. See [LICENSE-MIT](LICENSE-MIT) and [LICENSE-APACHE](LICENSE-APACHE).

Contributions are dual-licensed under the same terms unless explicitly stated otherwise.
