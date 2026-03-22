# iris — Roadmap

Context cache controller for LLM agents, implemented as a Rust MCP server.

---

## Phase P0: Foundation

### Tasks

- [x] Initialize Cargo workspace with iris-core, iris-mcp, iris-cli crates and edition = "2024"
- [x] Add #![deny(unsafe_code)] to all crate roots and configure workspace-level dependencies
- [x] Configure clippy (pedantic + deny warnings), rustfmt, cargo-deny, and cargo-audit
- [x] Create justfile with build, test, lint, fmt, coverage, audit, deny, and validate recipes
- [x] Set up GitHub Actions CI workflow calling just validate on push/PR
- [x] Define core error types with thiserror — IndexError, SessionError, StorageError, ParseError in iris-core
- [x] Set up tracing infrastructure with tracing-subscriber, EnvFilter, and structured JSON output
- [x] Define core domain types: ContentId, SectionId, ClaimId, Resolution enum, DocumentTree, Section, Claim
- [x] Add miette integration to iris-cli and iris-mcp for user-facing diagnostic errors
- [x] Create config.toml schema and loader for global iris configuration (~/.iris/config.toml)
- [x] Write unit tests for error types, config loading, and domain type construction

---

## Phase P1: Multi-Resolution Index

### Tasks

- [x] Create ~/.iris/corpora/<name>/ on-disk layout with meta.toml, content.db, sessions/
- [x] Implement SQLite schema — documents, sections, claims, summaries tables with parent-child relationships
- [x] Add file_hashes table for tracking source file content hashes (incremental re-indexing)
- [x] Configure WAL journal mode, NORMAL synchronous, and busy timeout for concurrent reads
- [x] Implement Storage trait in iris-core with CRUD operations for documents, sections, claims
- [x] Wrap rusqlite Connection with tokio::spawn_blocking for async-safe database access
- [x] Add schema migration system (versioned migrations, forward-only) for future upgrades
- [x] Implement corpus configuration (meta.toml) — source directories, embedding model choice, parser settings
- [x] Write integration tests against real SQLite — CRUD, concurrent reads, WAL behavior, migration rollforward

---

## Phase P2: Session Intelligence

### Tasks

- [x] Define DocumentParser trait — parse(path) -> Result<DocumentTree> with format-agnostic interface
- [x] Implement MarkdownParser using comrak AST — walk nodes to build structural section tree
- [x] Generate stable hierarchical section IDs from heading paths (e.g., docs/auth.md#3-2-error-handling)
- [x] Preserve code blocks, tables, and lists as typed structural nodes within sections
- [x] Implement heuristic claim extraction — sentence splitting, named entity filtering, assertion detection
- [x] Implement extractive summary generation — TF-IDF top-k sentence selection per section and per document
- [x] Build ingestion pipeline orchestrator — parse, section, extract claims, summarize, store to SQLite
- [x] Handle edge cases: documents without headings (paragraph-boundary splitting), empty sections, nested lists
- [x] Implement incremental re-indexing — compare file hashes, re-parse only changed files, update only changed sections
- [x] Add token counting utility (cl100k_base compatible) for accurate budget tracking on all content units
- [x] Write tests for Markdown parsing — heading hierarchy, code blocks, tables, GFM extensions, frontmatter
- [x] Write tests for claim extraction quality — precision/recall on a hand-labeled test corpus

---

## Phase P3: Polish & Release

### Tasks

- [x] Define Embedder trait in iris-core — embed(texts) -> Result<Vec<Vec<f32>>> with batch support
- [x] Implement FastEmbedder using fastembed crate with all-MiniLM-L6-v2 (384d) via ONNX Runtime
- [x] Add model download and caching — first-run model fetch with progress, cached in ~/.iris/models/
- [x] Implement configurable model selection — support swapping embedding models via corpus meta.toml
- [x] Define VectorIndex trait — insert, search_knn, delete, persist, load operations
- [x] Implement HnswIndex using hnswlib-rs — decoupled graph/storage, cosine similarity, configurable M and ef
- [x] Add memory-mapped persistence for the HNSW index (vectors.hnsw + vectors.meta) via memmap2
- [x] Embed all three resolution levels at ingestion — summaries, sections, and claims get separate vectors
- [x] Build multi-resolution query pipeline — search across summary/section/claim levels, merge and rank results
- [x] Implement resolution-aware result scoring — weight results by resolution level and query specificity
- [x] Add incremental vector index updates — insert/delete embeddings for changed sections without full rebuild
- [x] Write benchmarks for embedding throughput (docs/sec) and search latency (p50/p99) at 1k/10k/100k sections
- [x] Write tests for vector index — insert/search/delete correctness, persistence round-trip, concurrent reads

---

## Phase P4: MCP Server & Core Tools ✦ "Wire up rmcp and expose iris_survey, iris_read, iris_extract as MCP tools"

**Problem:** The agent needs a standards-compliant MCP interface to discover, survey, read, and extract context from the index — the progressive disclosure model that replaces one-shot RAG

**Solution:** Implement ServerHandler via rmcp with stdio transport, exposing the three core tools (iris_survey, iris_read, iris_extract) plus iris://status and iris://corpus resources. Include budget_status in every response

### Tasks

- [x] Implement ServerHandler trait via rmcp with stdio transport and #[tool] macro-based tool registration
- [x] Wire up iris-cli binary entry point — argument parsing (clap), corpus path, config loading, server startup
- [x] Implement iris_survey tool — vector search over section embeddings, return ranked summaries with relevance scores
- [x] Implement iris_read tool — full section text by hierarchical ID with heading_path and claims_available count
- [x] Implement iris_extract tool — claim-level retrieval within a specific section, filtered by query relevance
- [x] Add budget_status object to every tool response — tokens_used, tokens_remaining, pressure_level
- [x] Expose MCP resources — iris://status (index stats) and iris://corpus/{path} (document metadata)
- [x] Add #[instrument] tracing spans to all MCP tool handlers with request/response logging at DEBUG level
- [x] Implement graceful error handling — map iris-core errors to MCP ErrorData with user-friendly messages
- [x] Write end-to-end integration test — start MCP server, send JSON-RPC tool calls, verify responses
- [x] Test with real MCP client (Claude Code) — verify tool discovery, survey/read/extract flow on a sample corpus

---

## Phase P5: Session Shadow & Budget Management ✦ "Track what the agent has, estimate what it lost, manage the budget"

**Problem:** No existing retrieval system knows what context the agent already has. Without session state, iris would re-deliver identical content every turn — the worst possible use of scarce context window tokens

**Solution:** Implement the session shadow (delivered items + window estimation), deduplication, fault-based eviction correction, budget tracking with pressure mode, and the iris_budget/iris_compress/iris_evicted tools

### Tasks

- [x] Implement Session struct — id, created_at, agent_context_budget, delivered BTreeMap, trajectory vector
- [x] Implement DeliveredItem tracking — content_id, resolution, token_count, turn_delivered, content_hash
- [x] Build window estimation model — cumulative token tracking with configurable FIFO/LRU eviction assumption
- [x] Implement deduplication — compare incoming results against session shadow, skip already-delivered content
- [x] Implement delta updates — detect when a previously-delivered section has changed, return only the diff
- [x] Implement fault-based correction — detect re-requests as eviction signals, update window estimate
- [x] Implement iris_evicted tool — accept explicit agent feedback on dropped content_ids
- [x] Build budget tracker — configurable max_context_tokens, threshold-based pressure mode (default 80%)
- [x] Implement pressure mode behavior — auto-compress responses to claim-level, attach eviction recommendations
- [x] Implement eviction ranking — score delivered content by recency, relevance decay, and dependency graph
- [x] Implement iris_budget tool — return total_budget, estimated_used, pressure_level, eviction_candidates
- [x] Implement iris_compress tool — generate compressed summaries for content the agent wants to evict
- [x] Add session persistence to SQLite — save/restore session shadows for crash recovery
- [x] Write exhaustive tests for session shadow — deduplication, fault correction, window estimation accuracy
- [x] Write tests for budget manager — pressure mode transitions, eviction ranking, compression token savings

---

## Phase P6: Prefetch Engine & Coherence ✦ "Predict what the agent needs next and notify when source documents change"

**Problem:** Every cold retrieval costs 50-200ms and a tool-call round-trip. Without prefetching, agents make 3x more tool calls than necessary. Without coherence, stale context causes hallucinations

**Solution:** Build three prefetch heuristics (sequential, topical, structural locality) with an LRU warm cache, file watching via notify crate for coherence alerts, and the iris_related tool for claim dependency traversal

### Tasks

- [x] Implement sequential prefetch — when agent reads section N, pre-warm section N+1 and parent summary
- [x] Implement topical prefetch — maintain running topic vector from last K sections, pre-warm nearest un-accessed sections
- [x] Implement structural prefetch — pre-warm sibling sections and cross-referenced sections from document tree
- [x] Build LRU prefetch cache (default 50 items) with pre-computed text, token count, and relevance score
- [x] Wire prefetch into tool response path — serve from warm cache (<1ms) or fall through to cold retrieval
- [x] Add prefetch hit rate metrics — track warm/cold responses per session, expose via iris://status resource
- [x] Implement iris_related tool — claim dependency traversal (references, contradicts, depends_on, updates)
- [x] Build claim relationship index — detect cross-references and co-occurring entities between claims at ingestion
- [x] Implement file watcher using notify crate — watch corpus source directories for changes
- [x] Build coherence protocol — on file change, re-index affected sections, generate coherence_alert notifications
- [x] Send MCP notifications for stale content — push changed_sections and stale_content_ids to connected agents
- [x] Invalidate session shadow entries when underlying content changes — mark stale, offer delta on next access
- [x] Write tests for prefetch engine — hit rate measurement, sequential/topical/structural prediction accuracy
- [x] Write tests for coherence — file change detection, re-indexing correctness, notification delivery, shadow invalidation

---

## Phase P7: Polish, Parsers & Release ✦ "Additional format support, cross-session analytics, docs, benchmarks, and v0.1.0"

**Problem:** Markdown-only limits usefulness. Without benchmarks and documentation, adoption is blocked. Without cross-session learning, prefetch never improves beyond session-local heuristics

**Solution:** Add HTML and PDF parsers, cross-session analytics for prefetch tuning, comprehensive mdBook documentation, a reproducible benchmark suite, pre-built binaries for all platforms, and cut the v0.1.0 release

### Tasks

- [x] Implement HtmlParser using scraper + html2text — extract sections from semantic HTML (h1-h6, article, section tags)
- [x] Implement PdfParser using pdf-extract or lopdf — page-boundary splitting, heading detection from font size
- [x] Add parser auto-detection — select parser based on file extension, with manual override in corpus config
- [x] Implement cross-session analytics — track frequently-accessed sections, co-access patterns, per-corpus statistics
- [x] Feed cross-session data into prefetch — prioritize frequently-accessed and co-accessed sections for pre-warming
- [x] Build mdBook documentation site — architecture overview, getting started guide, tool reference, configuration
- [x] Write doc tests for all public APIs in iris-core — non-trivial examples with # Examples sections
- [x] Create reproducible benchmark suite — ingestion throughput, search latency, prefetch hit rate across corpus sizes
- [x] Build sample evaluation corpus — curated set of Markdown/HTML/PDF docs with ground-truth retrieval annotations
- [x] Set up cross-compilation CI — pre-built binaries for Linux x86_64, Linux aarch64, macOS Apple Silicon, Windows
- [x] Add installation methods — cargo install, homebrew tap, GitHub release binaries with checksums
- [x] Write MCP client configuration examples — Claude Code, Cursor, and generic JSON-RPC client setup guides
- [x] Final audit — cargo audit, cargo deny check, full test suite, clippy clean, benchmark baselines recorded
- [x] Tag and publish v0.1.0 release — changelog, GitHub release with binaries, crates.io publish

---

## Phase I0: Ingestion Pipeline Integration ✦ "Make iris actually read the corpus on startup"

**Problem:** iris starts with an empty database — corpus files are never parsed or indexed on startup, making all tools return empty results.

**Solution:** Wire ingest_directory_with_embeddings into the CLI startup path, persist the vector index after ingestion, and implement incremental re-indexing via file hashes.

### Tasks

- [x] Call ingest_directory_with_embeddings in CLI main.rs after initializing storage/embedder/index
- [x] Pass the user-provided --corpus path through to the ingestion pipeline (currently only used for corpus_name hashing)
- [x] Persist the HNSW vector index to disk after ingestion completes (call index.persist(&index_dir))
- [x] Implement incremental ingestion — skip files whose SHA-256 hash matches the stored hash, only re-index changed files
- [x] Log ingestion stats on startup (files scanned, sections created, embeddings generated, time elapsed)

---

## Phase I1: Server Lifecycle & Persistence ✦ "Sessions and analytics survive restarts"

**Problem:** CLI uses IrisServer::new() which creates ephemeral sessions with no analytics — session state and cross-session learning are lost on restart.

**Solution:** Switch CLI to IrisServer::with_persistence(), pass corpus path and storage through, enable session restore and analytics.

### Tasks

- [x] Switch CLI from IrisServer::new() to IrisServer::with_persistence(), passing Arc&lt;SqliteStorage&gt; and budget config
- [x] Generate or restore a stable session ID per corpus (derive from corpus path hash) so sessions persist across restarts
- [x] Load budget config from IrisConfig (config.toml) instead of using BudgetConfig::default()
- [x] Verify analytics co-access patterns are recorded and served back via iris_budget prefetch_metrics

---

## Phase I2: Coherence & File Watching ✦ "Detect file changes and keep the index fresh"

**Problem:** When corpus files change on disk, iris doesn't notice — stale content is served without alerts, and the index drifts from reality.

**Solution:** Spawn the FileWatcher and CoherenceEngine on startup, wire coherence events into session invalidation and re-ingestion.

### Tasks

- [x] Spawn FileWatcher on the corpus directory at CLI startup and feed events to CoherenceEngine
- [x] Wire CoherenceEngine.process_events to trigger re-ingestion of changed files and update the vector index
- [x] Propagate coherence alerts to active sessions via Session::invalidate_sections so stale content is flagged
- [x] Surface coherence_alerts in iris_read and iris_budget MCP tool responses when content has changed

---

## Phase I3: End-to-End Validation ✦ "Prove it actually works from CLI to MCP response"

**Problem:** All 99 roadmap tasks were marked done but the system doesn't work end-to-end — need integration tests that prove the full pipeline.

**Solution:** Write E2E tests that start iris against a real corpus, verify ingestion populates the DB, and validate each MCP tool returns real results.

### Tasks

- [ ] E2E test: start iris against a temp corpus dir, verify iris_survey returns ranked results
- [ ] E2E test: verify iris_read returns full section text with correct heading paths and content hashes
- [ ] E2E test: verify iris_extract returns claims from ingested content
- [ ] E2E test: verify iris_related follows claim dependency chains
- [ ] E2E test: verify session deduplication — iris_read same section twice returns skip/delta, not full content
- [ ] E2E test: verify iris_compress + iris_evicted cycle works and budget updates accordingly
- [ ] E2E test: modify a corpus file, verify coherence detects the change and iris_read returns updated content

