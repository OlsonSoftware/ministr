# Changelog

All notable changes to iris will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

#### Code navigation
- `iris_symbols`, `iris_definition`, `iris_references` — code symbol index across 12 languages via tree-sitter
- `iris_bridge` — cross-language bridge detection for Tauri, napi, PyO3, wasm-bindgen, and HTTP routes

#### Retrieval
- Two-stage Matryoshka retrieval with adaptive dimension selection
- SPLADE sparse embeddings + dense vectors with reciprocal rank fusion
- Cross-encoder reranking with configurable pipeline depth
- Candle Metal GPU embedding backend (optional, Apple Silicon)

#### Session & eviction
- Attention-position-aware eviction scoring (Lost in the Middle bias)
- FSRS spaced-repetition memory model for context retention
- Multi-tier compression with pluggable strategies and quality scoring

#### Multi-source corpora
- `iris_fetch` — fetch and index web content
- `iris_clone` — clone and index git repositories
- `iris_refresh` — detect and re-fetch stale sources

#### Architecture
- `iris-daemon` — HTTP API over Unix domain socket
- `iris-api` — shared request/response types and `DaemonClient`
- `iris-app` — Tauri v2 desktop app with system tray and dashboard
- `/activity` endpoint with `?limit` and `?since` for incremental polling
- Automatic stdio-to-HTTP proxy when a second client connects
- Streamable HTTP transport for remote deployments
- Concurrent producer-consumer ingestion pipeline

#### MCP protocol
- Structured output schemas and tool annotations for all tools
- Elicitation prompts for budget, compression, and search decisions
- Async tasks for long-running operations (fetch, clone, index)

#### Distribution
- Docker image with cargo-chef cached builds
- Fly.io and Railway deployment configs with Caddy/nginx templates
- Signed and notarized macOS `.pkg` installer
- `iris init` — project scaffolding with `.iris.toml` and MCP client configs
- Hot-reload on `.iris.toml` changes
- Retrieval evaluation suite with MRR/nDCG and CI regression gate

#### Desktop app
- **Cache observatory overhaul** — Overview home with aggregate budget ring, cache-hit history, and live turn stream
- `BudgetRing`, `TurnBlock`, `CorpusChip`, `StatusDot`, and `ActivityFeed` UI primitives
- Command palette (`⌘K`) with corpus navigation and theme/tab actions
- Keyboard shortcut sheet (`?`) and theme toggle (System / Dark / Light) in the TopBar
- Tray submenus for active sessions, recent corpora, and quick actions
- Live tool-call **activity stream** — every `iris_*` MCP call is recorded in a 500-event in-memory ring buffer on the daemon, exposed via `GET /activity` and surfaced in the app Overview
- Onboarding "observatory preview" tile so users see the dashboard aesthetic before reaching the dense dashboard
- `CorpusTreemap` re-themed with OKLCH language colors that share the iris design tokens

#### Documentation
- New documentation site built with Material for MkDocs, hosted on GitHub Pages
- Tailwind v4 design tokens (`docs/styles/tokens.css`) compiled into Material's theme
- Inline Phosphor icon sprite via a build-time MkDocs hook (`docs/hooks/inline_icons.py`)
- D2 diagrams throughout the architecture docs (replacing ASCII art)
- Animated Chart.js benchmarks, Cmd+K search, reading-progress indicator, and keyboard shortcuts on the docs site
- VHS terminal demo on the landing page

### Changed

- Workspace expanded from 3 crates to 6 (`iris-api`, `iris-daemon`, `iris-app`)
- Prefetch engine overhauled — `PriorityCache`, adaptive alpha, cache invalidation
- Ingestion pipeline split into focused submodules
- Session budget tracking integrated into daemon for cross-session awareness
- Documentation site migrated from mdBook to Material for MkDocs
- User-facing copy clarified: iris manages what it sends into context, it does
  not edit the agent's context window

### Fixed

- Navigation router on the docs site no longer breaks on SVG `<use>` elements;
  icons are inlined at build time so Material's URL normalizer has no
  `SVGUseElement.href` to trip on

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

[0.1.0]: https://github.com/AlrikOlson/iris-rs/releases/tag/v0.1.0
