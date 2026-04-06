# Launch Posts

Draft templates for community announcements. Customize each post for the specific audience and platform norms.

---

## Hacker News — Show HN

**Title**: `Show HN: iris – MCP server that manages LLM agent context like a CPU cache controller`

**Post**:

iris is a Rust-native MCP server that gives LLM coding agents (Claude Code, Cursor, etc.) structured, token-efficient access to codebases.

The problem: agents using grep+cat dump entire files into their context window. A 500-file project burns through 80K tokens in a few queries. Context fills up, the agent loses track of earlier information, and quality degrades.

iris treats the context window like a CPU cache:
- **Multi-resolution index** — parses code into sections + atomic claims, builds an HNSW vector index for semantic search (<60ms queries)
- **Session shadow** — tracks what each session has already seen, deduplicates re-reads, delivers only deltas when content changes
- **Prefetch engine** — predicts what the agent needs next based on access patterns and dependency graphs
- **Budget manager** — monitors token usage, recommends evictions, compresses stale content

Result: 90%+ token savings vs grep+cat workflows. The agent can explore a large codebase without running out of context.

Written in Rust, runs locally, uses the MCP protocol so it works with any compatible client. Metal GPU acceleration on Apple Silicon for embeddings.

```
cargo install iris-cli
cd your-project && iris init
```

GitHub: [link]
Docs: [link]
Benchmarks: [link]

---

## r/rust

**Title**: `iris: a Rust MCP server for LLM agent context management — session tracking, semantic search, budget control`

**Post**:

I've been building iris, an MCP server in Rust that manages LLM agent context windows. Sharing it here because:

1. It's a Rust workspace (edition 2024) with some interesting architectural patterns
2. The Rust ecosystem doesn't have many MCP servers yet
3. I'd love feedback from the community

**What it does**: LLM coding agents (Claude Code, Cursor) need to read your codebase, but they waste tokens dumping entire files into context. iris indexes your code into semantic sections, tracks session state, and delivers only what the agent actually needs.

**Architecture**:
- `iris-core` — domain logic, no transport dependencies. HNSW vector index, session shadow, prefetch engine, budget manager
- `iris-mcp` — MCP protocol adapter via `rmcp`
- `iris-daemon` — HTTP API over Unix domain socket
- `iris-app` — Tauri v2 desktop app (in progress)
- `iris-cli` — binary entry point

**Rust-specific notes**:
- Edition 2024 (Rust 1.85+)
- `#![deny(unsafe_code)]` in every crate — relies on safe abstractions from fastembed, rusqlite, memmap2
- `thiserror` for library errors, `miette` for CLI diagnostics
- Candle for embeddings with Metal backend on Apple Silicon
- Layered architecture: transport -> service -> storage, no layer skipping

**Performance**: Sub-60ms end-to-end query latency. HNSW search is <10ms for 10K sections. 90%+ token savings vs raw file reading.

`cargo install iris-cli` — feedback welcome.

GitHub: [link]

---

## r/programming

**Title**: `iris: treating the LLM context window as a cache — session tracking, prefetch, and budget management for coding agents`

**Post**:

I built an MCP server that applies CPU cache management principles to LLM context windows.

**The insight**: an LLM agent's context window has the same fundamental constraints as a CPU cache — limited capacity, variable access patterns, significant cost for cache misses (re-reading content), and benefit from locality (related code is accessed together). But current tools treat it as an infinite append-only log.

**iris applies cache management**:
- **Session shadow** = cache tag directory. Tracks every section the agent has consumed. Re-reads return nothing (hit). Changed content returns only the delta (partial hit).
- **Prefetch engine** = hardware prefetcher. When the agent reads a function, iris pre-loads its callers and referenced types before the agent asks for them.
- **Budget manager** = cache replacement policy. Monitors usage, recommends eviction of stale sections, compresses evicted content into summaries that preserve key facts.

The index layer uses HNSW for vector search over section embeddings, plus a symbol table for structural code navigation (definitions, references, callers).

Result: agents using iris consume 90%+ fewer tokens than grep+cat workflows, and the effective "hit rate" improves over the course of a session as the prefetch engine learns access patterns.

Written in Rust, uses the MCP protocol, works with Claude Code / Cursor / any MCP client.

GitHub: [link]
Benchmarks: [link]

---

## r/LocalLLaMA

**Title**: `iris: MCP server that makes local LLM agents smarter about codebase navigation — 90%+ token savings`

**Post**:

If you're running local LLMs for coding tasks, context window size is your biggest constraint. iris is an MCP server that gives your agent semantic, token-efficient access to codebases.

**Why this matters for local LLMs**:
- Local models often have 8K-32K context windows — every token counts
- iris delivers targeted sections (~100 tokens) instead of entire files (~2000 tokens)
- Semantic search means the model doesn't need to formulate perfect grep queries
- Session tracking means no wasted tokens on re-reading the same code

**How it works**:
1. `iris init` indexes your project (embeddings via Candle — runs locally, no API calls)
2. Your agent calls iris tools via MCP: `iris_survey("authentication flow")` returns the 10 most relevant code sections
3. Session shadow deduplicates — if the agent already saw a section, iris says so instead of resending it

Runs entirely locally. Metal GPU acceleration on Apple Silicon, CPU fallback everywhere else. Embeddings use `bge-small-en-v1.5` (33M params, ~50ms per query on Metal).

`cargo install iris-cli`

GitHub: [link]

---

## r/ClaudeAI

**Title**: `iris: MCP server that gives Claude structured codebase access with 90%+ token savings`

**Post**:

I built an MCP server specifically designed for how Claude agents interact with codebases.

**The problem**: When Claude Code reads your project, it uses grep+cat to dump files into context. This works for small projects but scales badly — a 500-file codebase can burn 80K tokens in a few queries, and Claude starts losing track of earlier content.

**iris fixes this**:
- Indexes your codebase into semantic sections with vector embeddings
- Claude asks natural language questions ("how does auth work?") and gets precisely the relevant code sections
- Tracks what Claude has already seen in this session — no duplicate content
- Manages the context budget — recommends what to evict when context gets full

**Setup with Claude Code**:
```
cargo install iris-cli
cd your-project
iris init
```

Then add iris to your Claude Code MCP config. Claude automatically discovers and uses the iris tools.

**What changes for Claude**:
- Instead of `Read file.rs` (2000 tokens), Claude calls `iris_survey("error handling")` (400 tokens, better targeted)
- Instead of re-reading a file it saw earlier, iris tells Claude "you already have this" (0 tokens)
- Instead of guessing which files to read, the semantic index finds relevant code Claude didn't know existed

GitHub: [link]

---

## r/cursor

**Title**: `iris: MCP server for token-efficient codebase access — works with Cursor`

**Post**:

iris is an MCP server that indexes your codebase and gives Cursor's agent structured, semantic access. Instead of reading entire files, the agent gets targeted sections ranked by relevance.

Works via the MCP protocol — add it to your Cursor MCP config and the tools are automatically available.

Key features: semantic search, session dedup (no re-reading), budget management, symbol navigation (definitions/references/callers).

`cargo install iris-cli && cd your-project && iris init`

GitHub: [link]

---

## MCP Registry Listings

### awesome-mcp-servers

**Category**: Developer Tools / Code Intelligence

```markdown
- [iris](https://github.com/iris-rs/iris-rs) - Context window management for LLM agents.
  Indexes codebases into semantic sections, tracks session state, manages context budget.
  Session shadow deduplicates re-reads, prefetch engine predicts agent needs,
  budget manager recommends evictions. 90%+ token savings vs raw file reading.
  Rust, Metal GPU, <60ms queries.
```

### MCP Market / Smithery / mcp.so

**Name**: iris

**Short description**: MCP server that manages LLM agent context windows like a CPU cache controller — with session tracking, semantic search, prefetch, and budget management.

**Long description**:

iris gives LLM coding agents structured, token-efficient access to codebases. Instead of reading entire files, agents query a semantic index and receive precisely the relevant sections.

Core capabilities:
- Semantic search over code and docs via HNSW vector index (<60ms queries)
- Session shadow tracks consumed content, deduplicates re-reads, delivers deltas
- Prefetch engine predicts agent needs from access patterns and dependency graphs
- Budget manager monitors context usage, recommends evictions, compresses stale content
- Code intelligence: symbol definitions, references, callers across the full codebase

90%+ token savings vs grep+cat workflows. Written in Rust with Metal GPU acceleration.

**Tags**: code-intelligence, context-management, semantic-search, rust, developer-tools

**Install**:
```
cargo install iris-cli
```

**Tools provided**:
- `iris_survey` — semantic search
- `iris_symbols` — symbol search
- `iris_definition` — symbol definitions
- `iris_references` — find references
- `iris_read` — read sections (with dedup)
- `iris_extract` — atomic claims
- `iris_related` — dependency chains
- `iris_budget` — context budget status
- `iris_compress` — compress sections
- `iris_toc` — corpus overview
