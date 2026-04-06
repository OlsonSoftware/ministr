# iris

**MCP server that manages agent context windows like a CPU cache controller.**

LLM agents waste most of their context window reading entire files when they only need a few functions. iris indexes your codebase into semantic sections, tracks what each session has already seen, predicts what it will need next, and delivers precisely-targeted content — saving 90%+ tokens on every query.

---

## Features

| Feature | What It Does |
|---------|-------------|
| **Multi-Resolution Index** | Parses source code and docs into sections, extracts atomic claims, builds HNSW vector index for sub-10ms semantic search |
| **Session Shadow** | Tracks exactly which sections each agent session has consumed — deduplicates on re-reads, delivers only deltas when content changes |
| **Prefetch Engine** | Predicts what the agent will need next based on access patterns and dependency graphs, pre-loads it before the agent asks |
| **Budget Management** | Monitors context window usage, recommends evictions, compresses stale content — keeps the agent operating within its token budget |
| **Code Intelligence** | Symbol-level navigation: find definitions, references, callers, implementors across your entire codebase via `iris_symbols`, `iris_definition`, `iris_references` |
| **Semantic Search** | Natural language queries find conceptually relevant code that grep misses — "how does error handling work" returns actual error handling code, not string matches |

## Install

```sh
cargo install iris-cli
```

Requires Rust 1.85+ (edition 2024). Metal GPU acceleration is enabled automatically on Apple Silicon.

## Quick Start

```sh
# Index your project
cd your-project
iris init

# Start your MCP-compatible agent (Claude Code, Cursor, etc.)
# iris tools are automatically available:
#   iris_survey  — semantic search
#   iris_symbols — find code symbols
#   iris_read    — read sections (with dedup)
#   iris_extract — get atomic claims
#   iris_budget  — check context usage
```

That's it. Your agent now has structured, token-efficient access to your entire codebase.

## How It Works

1. **`iris init`** indexes your project — parses files into sections, generates embeddings, builds the HNSW search index
2. **Your agent calls iris MCP tools** instead of reading raw files — `iris_survey("error handling")` returns the 10 most relevant sections, ranked by semantic similarity
3. **The session shadow tracks consumption** — if the agent re-reads a section, iris returns nothing (already in context). If the section changed since last read, iris returns only the delta
4. **The prefetch engine anticipates needs** — when the agent reads a function, iris pre-loads its callers and the types it references
5. **The budget manager watches token usage** — when context pressure rises, iris recommends which stale sections to evict and offers compressed summaries

## Comparison

| Capability | grep + cat | Basic RAG | **iris** |
|-----------|-----------|-----------|---------|
| Token efficiency | Entire files (~2000 tok/file) | Chunks (~500 tok) | Targeted sections (~100 tok) |
| Session awareness | None | None | Full dedup + delta delivery |
| Predictive loading | None | None | Dependency-based prefetch |
| Budget management | None | None | Eviction + compression |
| Code navigation | Text search only | Embedding search | Symbols + embeddings + references |
| Query latency | ~1ms | ~100ms | **~60ms** |
| Conceptual queries | String match only | Semantic | **Semantic + structural** |
| Setup complexity | None | Vector DB + pipeline | **`iris init`** |

## MCP Tools

iris exposes its capabilities as standard MCP tools, compatible with any MCP client:

| Tool | Purpose |
|------|---------|
| `iris_survey` | Semantic search across docs and code |
| `iris_symbols` | Find structs, functions, traits, enums by name/kind/module |
| `iris_definition` | Get full source definition of a symbol |
| `iris_references` | Find callers, implementors, importers |
| `iris_read` | Read a section with dedup and delta delivery |
| `iris_extract` | Get atomic claims, optionally filtered |
| `iris_related` | Follow dependency chains between claims |
| `iris_budget` | Check context budget and eviction recommendations |
| `iris_compress` | Generate compressed summaries for eviction |
| `iris_toc` | Structural overview of the indexed corpus |

## Supported Clients

iris works with any MCP-compatible client:

- **Claude Code** (Anthropic CLI)
- **Claude Desktop**
- **Cursor**
- **Windsurf**
- **Any client implementing the MCP specification**

## Links

- [GitHub Repository](https://github.com/iris-rs/iris-rs)
- [Architecture Overview](architecture.md)
- [Getting Started Guide](getting-started.md)
- [Benchmarks](benchmarks.md)
- [Configuration Reference](configuration.md)
