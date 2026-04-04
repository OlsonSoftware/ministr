# Launch Post Drafts

## OSS2.3 — Show HN

**Title:** Show HN: iris -- MCP server that traces code across language boundaries (Rust)

**Body:**

Hi HN, I built iris, an MCP server that manages LLM agent context windows like a CPU cache controller.

The problem: when LLM agents work on large codebases, they waste context tokens re-reading files, lose track of what they've already seen, and can't follow function calls across languages (e.g., TypeScript invoke() -> Rust handler in a Tauri app).

iris solves this with:

- **Multi-resolution semantic index** — documents, sections, claims, and code symbols all searchable by meaning
- **Cross-language bridge tracing** — automatically maps function calls across Rust/TypeScript/Python/Go/etc. boundaries (found 52 Tauri command links in a real project)
- **Session shadow** — tracks what the agent has seen, deduplicates, and manages a token budget with pressure-based eviction
- **Predictive prefetch** — warms the cache based on sequential, topical, and structural locality
- **Live coherence** — file watcher re-indexes on change, alerts the agent about stale content

Written in Rust, ~20 MB memory for the MCP proxy (vs 2 GB+ for the full server). Runs as a daemon with a Tauri desktop app for project management.

Tech: Rust, tree-sitter (12 languages), HNSW vector index, SQLite, fastembed, axum, rmcp

GitHub: https://github.com/alrik/iris-rs

---

## OSS2.4 — Reddit Posts

### r/rust

**Title:** iris: MCP server for LLM agent context management, written in Rust

I've been building iris, an MCP server that helps LLM agents navigate large codebases efficiently. It's written entirely in Rust and uses tree-sitter for code parsing, HNSW for vector search, SQLite for storage, and axum for the daemon API.

The highlight feature is cross-language bridge tracing — it automatically maps function calls across language boundaries (e.g., Tauri's `invoke("cmd")` in TypeScript to `#[tauri::command]` in Rust). On a real Tauri project, it found 52 bridge links automatically.

Architecture: iris-core (domain logic) / iris-daemon (axum HTTP on UDS) / iris-mcp (MCP protocol) / iris-app (Tauri desktop). The daemon runs ~20 MB, the full server with embeddings ~2 GB.

Workspace: 5 crates, ~15k lines of Rust, 1600+ tests. Uses fastembed for local embeddings (no API keys).

https://github.com/alrik/iris-rs

### r/ClaudeAI

**Title:** Built an MCP server that manages Claude's context window like a CPU cache

iris is an MCP server I built that sits between Claude (Code, Desktop, etc.) and your codebase. Instead of Claude re-reading files every time, iris:

1. Indexes your project into searchable sections, claims, and code symbols
2. Tracks what Claude has already seen (session shadow)
3. Manages a token budget — tells Claude when to compress or evict content
4. Prefetches content Claude is likely to need next
5. Traces function calls across languages (Rust/TypeScript/Python/etc.)

It runs locally, uses local embeddings (no API keys), and works with any MCP client. The desktop app lets you manage projects from the system tray.

https://github.com/alrik/iris-rs

---

## OSS2.5 — X/Twitter Thread

**Thread:**

1/ I built an MCP server that automatically maps every function call across language boundaries in your codebase.

On a Tauri app with Rust + TypeScript, it found 52 bridge links — every invoke() in TypeScript matched to its #[tauri::command] handler in Rust.

2/ The problem: LLM agents working on polyglot codebases can't follow calls across language boundaries. They see invoke("get_config") in TypeScript but don't know where the Rust handler lives.

iris solves this with tree-sitter + bridge link analysis.

3/ But that's just one feature. iris manages the full LLM context window:

- Semantic search across docs + code
- Token budget tracking with eviction
- Predictive prefetch (sequential, topical, structural)
- Live coherence when files change

4/ Architecture:
- Rust daemon on Unix domain socket (~20 MB)
- MCP proxy for Claude/Cursor/etc.
- Tauri desktop app for project management
- 12 languages, 1600+ tests, local embeddings

5/ It's open source:
https://github.com/alrik/iris-rs

MIT / Apache-2.0
