# Changelog

All notable changes to iris will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- **Code symbol index** — `iris_symbols`, `iris_definition`, `iris_references` tools for navigating structs, functions, traits, and enums across 12 languages via tree-sitter
- **Cross-language bridge detection** — automatic linking of Tauri commands, napi bindings, PyO3 functions, wasm-bindgen exports, and HTTP routes via `iris_bridge`
- **Daemon architecture** — `iris-daemon` crate providing HTTP API over Unix domain socket; `iris-api` crate for shared request/response types
- **Tauri desktop app** — `iris-app` with system tray, project management dashboard, and ingestion progress UI
- **MCP proxy mode** — automatic stdio-to-HTTP proxy when a second client connects to a running daemon
- **Streamable HTTP transport** — in addition to stdio, for containerized and remote deployments
- **Candle Metal GPU backend** — optional Metal-accelerated embeddings on Apple Silicon via candle
- **Two-stage Matryoshka retrieval** — coarse/fine reranking with adaptive dimension selection
- **Hybrid search** — SPLADE sparse embeddings + dense vectors with reciprocal rank fusion
- **Cross-encoder reranking** — pluggable reranker trait with configurable pipeline depth
- **Multi-source corpora** — index local directories, web URLs (`iris_fetch`), and git repositories (`iris_clone`) with `iris_refresh` for staleness detection
- **Concurrent ingestion** — producer-consumer embedding pipeline with progress notifications
- **Attention-position-aware eviction** — scoring that accounts for Lost in the Middle bias
- **FSRS memory model** — spaced-repetition-based context retention scoring
- **Multi-tier compression** — pluggable compression strategies with auto-tier selection and quality scoring
- **Structured output schemas** — tool annotations and JSON schema output for all MCP tools
- **MCP elicitation** — interactive prompts for budget decisions, compression options, and search refinement
- **MCP async tasks** — background execution for long-running operations (fetch, clone, index)
- **Docker image** — multi-stage build with cargo-chef for cached dependency layers
- **Fly.io / Railway deployment** — production configs with Caddy/nginx reverse proxy templates
- **macOS installer** — signed and notarized `.pkg` with custom resources
- **`iris init`** — project scaffolding with `.iris.toml`, `.mcp.json`, and agent config generation
- **Hot-reload** — `.iris.toml` changes trigger automatic re-indexing
- **Retrieval evaluation suite** — MRR, nDCG metrics with CI regression gate

### Changed

- Workspace expanded from 3 crates to 6 (`iris-api`, `iris-daemon`, `iris-app`)
- Prefetch engine overhauled with `PriorityCache`, adaptive alpha, and cache invalidation
- Ingestion pipeline split into focused submodules (discovery, embedding, processing, sections, symbols)
- Session budget tracking integrated into daemon for cross-session awareness

## [0.1.0] - 2026-03-21

### Added

- **MCP server** with stdio transport — 7 tools for LLM context management:
  - `iris_survey` — semantic search across a document corpus at multiple resolutions
  - `iris_read` — retrieve full section content with heading paths
  - `iris_extract` — extract atomic claims from sections, optionally ranked by query relevance
  - `iris_related` — follow dependency chains between claims (references, contradicts, depends_on, updates)
  - `iris_budget` — context budget status with eviction recommendations
  - `iris_compress` — generate compressed summaries for eviction candidates
  - `iris_evicted` — explicit eviction feedback from the agent
- **MCP resources** — `iris://status` for index/session state, `iris://corpus/{path}` for document metadata
- **Multi-resolution indexing** — documents, section summaries, section text, and atomic claims are embedded and indexed separately
- **Session shadow** — tracks what content has been delivered to the agent, deduplicates repeat deliveries, and detects fault-based evictions
- **Budget tracker** — estimates context window token usage, reports pressure levels, and ranks eviction candidates
- **Prefetch engine** — sequential, structural, topical, and cross-session prefetch strategies with LRU cache
- **Coherence subsystem** — file watcher triggers re-indexing and invalidates stale session entries
- **Cross-session analytics** — tracks section access patterns and feeds co-access data into prefetch
- **Session persistence** — session state survives server restarts via SQLite storage
- **Parsers** — Markdown (via comrak), HTML (via scraper), PDF (via pdf-extract), with auto-detection by file extension
- **Claim relationship index** — directed relationships between claims with confidence scores
- **Extractive summarization** — sentence-level extraction for compress and document summaries
- **HNSW vector index** — fast approximate nearest neighbor search (hnsw_rs)
- **FastEmbed embeddings** — local embedding model via fastembed (no API keys required)
- **CLI** — `iris` binary with `--corpus` and `--config` flags
- **Configuration** — TOML config file at `~/.iris/config.toml` with sensible defaults
- **Cross-platform builds** — CI produces binaries for Linux (x86_64, aarch64), macOS (aarch64), and Windows (x86_64)
- **Quality gates** — clippy pedantic, cargo-audit, cargo-deny, and full test suite in CI
- **mdBook documentation** — architecture guide, MCP client setup, and API reference

[0.1.0]: https://github.com/alrik/iris-rs/releases/tag/v0.1.0
