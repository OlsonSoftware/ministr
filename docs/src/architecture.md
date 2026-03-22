# Architecture Overview

## Workspace Structure

iris is organized as a Cargo workspace with three crates:

```
iris-core/     — domain logic, no transport dependencies
iris-mcp/      — MCP server, depends on iris-core + rmcp
iris-cli/      — binary entry point, depends on iris-mcp
```

**iris-core** contains all business logic: parsing, indexing, embedding, search, session management, prefetching, and budget tracking. It has no knowledge of MCP or any transport protocol.

**iris-mcp** adapts iris-core's service layer to the MCP protocol using the `rmcp` crate. It implements tool handlers, request routing, and response formatting.

**iris-cli** is the binary entry point. It parses CLI arguments, loads configuration, and starts the MCP server over stdio transport.

## Layered Architecture

Each crate follows strict transport → service → storage layering:

```
┌─────────────────────────────────────────────┐
│  Transport (iris-mcp)                       │
│  MCP tool handlers, JSON-RPC, req/res map   │
├─────────────────────────────────────────────┤
│  Service (iris-core)                        │
│  Session shadow, prefetch, budget, query    │
├─────────────────────────────────────────────┤
│  Storage (iris-core)                        │
│  SQLite, HNSW index, file system, mmap I/O  │
└─────────────────────────────────────────────┘
```

No layer may skip a level. Transport calls service; service calls storage. Storage never calls service; service never calls transport.

## Deployment Model

iris runs as a standalone sidecar process. Any MCP-compatible agent connects to it over stdio (or optionally HTTP):

```
┌──────────────────┐     MCP (JSON-RPC)     ┌──────────────────┐
│   Any LLM Agent  │ ◄───────────────────► │      iris        │
│                  │   tool calls/responses  │                  │
│  Claude Code     │                         │  Rust binary     │
│  Cursor          │                         │  ~30MB           │
│  Custom agent    │                         │  Single process  │
└──────────────────┘                         └────────┬─────────┘
                                                      │
                                             ┌────────▼─────────┐
                                             │  Document corpus │
                                             │  (local files)   │
                                             └──────────────────┘
```

## The Five Mechanisms

iris combines five mechanisms that map directly to CPU cache controller concepts:

### 1. Session Shadow
Tracks exactly what context has been delivered to the agent, enabling deduplication, delta updates, and eviction estimation. See [Session Shadow](concepts/session-shadow.md).

### 2. Multi-Resolution Index
Documents are indexed at three simultaneous levels — summaries, sections, and claims — preserving the author's original structure rather than destroying it with fixed-size chunking.

### 3. Progressive Disclosure
Four MCP tools (`survey` → `read` → `extract` → `related`) give the agent explicit control over retrieval depth, mirroring how a human researcher navigates a knowledge base.

### 4. Speculative Prefetch
Predicts what the agent will need next based on sequential, topical, and structural locality. Pre-computes results so warm responses are served in <1ms. See [Prefetch Engine](concepts/prefetch-engine.md).

### 5. Context Budget Management
Tracks cumulative token usage, detects pressure, and provides eviction recommendations with compressed replacement summaries. See [Budget Management](concepts/budget-management.md).

## On-Disk Layout

```
~/.iris/
├── config.toml                     # Global configuration
└── corpora/
    └── <corpus-name>/
        ├── meta.toml               # Corpus config
        ├── content.db              # SQLite: sections, claims, summaries
        ├── vectors.hnsw            # Memory-mapped HNSW index
        ├── vectors.meta            # Index metadata
        ├── file_hashes.json        # For incremental re-indexing
        └── sessions/
            └── <session-id>.json   # Persisted session shadows
```

## Key Dependencies

| Purpose | Crate |
|---|---|
| Embeddings | `fastembed` (ONNX-based, local) |
| Vector search | HNSW index (in-memory, memory-mapped) |
| Storage | `rusqlite` (SQLite) |
| MCP protocol | `rmcp` |
| Document parsing | `comrak` (Markdown), `scraper` (HTML), `pdf-extract` (PDF) |
| Async runtime | `tokio` |
| Observability | `tracing` |
| File watching | `notify` |
