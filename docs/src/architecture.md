# Architecture Overview

## Workspace Structure

iris is organized as a Cargo workspace with six crates:

```
iris-core/          — domain logic, no transport dependencies
iris-api/           — shared request/response types for daemon ↔ MCP/CLI communication
iris-daemon/        — HTTP API over Unix domain socket, depends on iris-core + iris-api
iris-mcp/           — MCP server adapter, depends on iris-core + iris-api + rmcp
iris-cli/           — binary entry point, depends on iris-mcp
iris-app/src-tauri/ — Tauri v2 desktop app, depends on iris-core + iris-api + iris-daemon
```

**iris-core** contains all business logic: parsing, indexing, embedding, search, session management, prefetching, and budget tracking. It has no knowledge of MCP or any transport protocol.

**iris-api** defines the shared request/response types and the `DaemonClient` for communicating with the iris daemon over a Unix domain socket (`~/.iris/irisd.sock`). It has no dependency on iris-core — it is pure types.

**iris-daemon** is the long-running background service that owns the heavy resources (ONNX model, HNSW indexes, SQLite). It exposes an HTTP API over UDS for corpus management and querying.

**iris-mcp** adapts iris-core's service layer to the MCP protocol using the `rmcp` crate. It implements tool handlers, request routing, and response formatting. Transitioning to a thin proxy that delegates storage/embedding/indexing to the daemon.

**iris-cli** is the binary entry point. It parses CLI arguments, loads configuration, and starts the MCP server over stdio transport.

**iris-app** is the Tauri v2 desktop application that wraps the daemon with a GUI dashboard and system tray icon.

## Layered Architecture

Each crate follows strict transport → service → storage layering:

```d2
direction: down

transport: Transport (iris-mcp) {
  tooltip: "MCP tool handlers, JSON-RPC, req/res map"
  style.fill: "#6366f1"
  style.font-color: "#ffffff"
}

daemon: Daemon API (iris-daemon) {
  tooltip: "HTTP over UDS, corpus routes, SSE streams"
}

service: Service (iris-core) {
  tooltip: "Session shadow, prefetch, budget, query"
}

storage: Storage (iris-core) {
  tooltip: "SQLite, HNSW index, file system, mmap I/O"
}

transport -> daemon
daemon -> service
service -> storage
```

No layer may skip a level. Transport calls service; service calls storage. Storage never calls service; service never calls transport.

## Deployment Model

iris uses a daemon + proxy architecture. The daemon (iris-app or iris-daemon) owns heavy resources and persists across sessions. MCP-compatible agents connect to a thin MCP proxy (iris-cli) over stdio, which delegates to the daemon via UDS:

```d2
direction: right

agents: Any LLM Agent {
  claude: Claude Code
  cursor: Cursor
  custom: Custom agent
}

proxy: iris-cli (MCP)\nthin proxy {
  shape: rectangle
}

daemon: iris-daemon\nONNX · HNSW · SQLite\nsessions {
  shape: rectangle
}

app: iris-app (GUI)\nsystem tray {
  shape: rectangle
}

corpus: Document corpus\n(local files) {
  shape: cylinder
}

agents.claude -> proxy: "MCP / JSON-RPC\n(stdio)"
agents.cursor -> proxy
agents.custom -> proxy
proxy -> daemon: UDS\n~/.iris/irisd.sock
app -> daemon: embeds
daemon -> corpus
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
├── corpora.json                    # Daemon's registered-corpus manifest
├── irisd.sock                      # Daemon Unix domain socket
├── irisd.pid                       # Daemon PID file
└── corpora/
    └── <corpus-id>/                # Stable hash derived from corpus paths
        ├── content.db              # SQLite: sections, claims, summaries,
        │                           # file hashes, and session shadows
        └── index/
            ├── iris_hnsw.hnsw.data # HNSW vector dump
            ├── iris_hnsw.hnsw.graph
            └── id_map.json         # Section ID ↔ vector slot mapping
```

File hashes used for incremental re-indexing live in `content.db` (the
`file_hashes` table), and session shadows are stored in the `sessions`
table of the same database — neither is a standalone file on disk.

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
