# iris Demo Recording Scripts

## Demo 1: 60-Second Terminal Demo (asciinema)

**Setup:** Clone squid2 (a Tauri app with Rust + TypeScript) into a temp dir.

```sh
# Record with: asciinema rec demo-60s.cast --rows 30 --cols 100

# 1. Show the config (2s)
cat .iris.toml

# 2. Index the project (5s — use --release)
iris index --corpus ./src-tauri/src ./src

# 3. Find bridge links across Rust↔TypeScript (10s)
iris bridge --corpus ./src-tauri/src ./src

# Expected output: ~52 Tauri command bridge links mapping
# invoke("command_name") in TypeScript ↔ #[tauri::command] in Rust

# 4. Drill into one link (5s)
iris symbols --query "get_config"
iris definition --id "sym-src-tauri/src/commands.rs::get_config"
iris references --id "sym-src-tauri/src/commands.rs::get_config"

# 5. Show it working as an MCP server (5s)
# (show .mcp.json config, then Claude Code using iris_survey)
```

**Narration overlay (if screen recording):**
> "iris indexes your codebase and traces code across language boundaries.
> Here it found 52 Tauri command links between Rust and TypeScript —
> automatically. Every invoke() call mapped to its #[tauri::command] handler."

---

## Demo 2: 2-Minute Deep Dive (screen recording)

**Script:**

```
00:00 — Title card: "iris — MCP server for LLM agent context"

00:05 — Show .iris.toml configuration
  cat .iris.toml
  # paths, ignore patterns, model selection

00:15 — Index the project
  iris index --corpus . --release
  # Show progress: files discovered, sections, embeddings

00:30 — Semantic search with iris_survey
  # In Claude Code session, type a question
  # Show iris_survey returning ranked results with relevance scores

00:45 — Code navigation with iris_symbols
  # Search for a struct by name
  # Show iris_definition returning full source
  # Show iris_references finding all callers

01:00 — Cross-language bridge trace
  # Show iris_bridge finding Tauri command links
  # Highlight: TypeScript invoke() ↔ Rust handler mapping
  # Show confidence scores

01:15 — Context budget management
  # Show iris_budget with token counts
  # Show iris_compress generating summaries
  # Show iris_evicted freeing context space

01:30 — Live coherence
  # Edit a file, show the watcher detecting changes
  # iris_survey returns updated content

01:45 — Summary card:
  "iris: semantic search, code symbols, cross-language bridges,
   budget management, live coherence — all through MCP"

02:00 — End card with GitHub URL + install command
```
