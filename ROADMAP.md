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

- [x] E2E test: start iris against a temp corpus dir, verify iris_survey returns ranked results
- [x] E2E test: verify iris_read returns full section text with correct heading paths and content hashes
- [x] E2E test: verify iris_extract returns claims from ingested content
- [x] E2E test: verify iris_related follows claim dependency chains
- [x] E2E test: verify session deduplication — iris_read same section twice returns skip/delta, not full content
- [x] E2E test: verify iris_compress + iris_evicted cycle works and budget updates accordingly
- [x] E2E test: modify a corpus file, verify coherence detects the change and iris_read returns updated content

---

## Phase I4: Corpus Table of Contents Tool ✦ "Let agents see the full corpus map in one call"

**Problem:** Agents can't browse what's in the corpus — they must guess query terms for iris_survey. If the first survey misses, the agent is blind to available content.

**Solution:** Add an iris_toc tool that returns the full document/section tree from storage. Reuse existing list_documents + list_sections APIs. Lightweight, no embedding needed — pure metadata query.

### Tasks

- [x] Add a toc() method to QueryService that calls storage.list_documents() + storage.list_sections(doc_id) for each doc, returns a Vec&lt;TocEntry&gt; tree
- [x] Define TocEntry struct in iris-core/types.rs: document_id, section_id, heading_path, depth, claims_available, token_count — no text content, metadata only
- [x] Add iris_toc MCP tool handler in server.rs with optional document_id filter param — returns full tree when no filter, single doc tree when filtered
- [x] Include corpus_stats (total docs, total sections, total claims) in the iris_toc response header for quick orientation
- [x] Unit test: toc() returns correct tree structure for multi-doc corpus with nested headings
- [x] E2E test: iris_toc via call_tool returns all documents and sections from the test fixture

---

## Phase I5: Adaptive Section Merging ✦ "Fewer, meatier chunks — fewer round trips"

**Problem:** Heading-based chunking produces many 1-2 sentence sections for small docs. Agents need 6+ reads to understand one concept, wasting round trips and token overhead on per-response JSON framing.

**Solution:** Add a post-parse merge pass in the ingestion pipeline: coalesce adjacent sibling sections below a configurable token threshold into their parent. Preserves heading structure for large docs, merges aggressively for small ones. NAACL 2025 research confirms fixed ~200-word chunks match semantic chunking — target that as the floor.

### Tasks

- [x] Add a configurable min_section_tokens threshold to CorpusConfig (default: 50 tokens ~200 words) — sections below this are merge candidates
- [x] Implement coalesce_small_sections(sections: Vec&lt;Section&gt;, min_tokens: usize) -> Vec&lt;Section&gt; in ingestion.rs — merges adjacent sibling sections (same depth) below threshold into their parent, concatenating text with heading markers
- [x] Preserve section IDs for merged sections: use the parent's section_id, store child heading_paths as sub-headings in the merged text so they remain searchable
- [x] Wire coalesce_small_sections into ingest_directory_with_embeddings after parse + split_large_headingless but before enrichment and embedding — single insertion point, no duplication
- [x] Unit test: 3 sibling sections of 10/15/8 tokens merge into 1; a 200-token section stays untouched; mixed depths merge correctly at each level
- [x] Unit test: merging preserves document order (position field) and updates claims_available counts on merged sections
- [x] Integration test: ingest the iris docs corpus, verify section count decreases vs unmerged, and verify survey still returns relevant results for queries that would have matched child headings

---

## Phase I6: Survey-Triggered Prefetch ✦ "Predict the obvious next read after every survey"

**Problem:** When survey returns claim-level hits, the agent almost always reads the parent section next. But prefetch only triggers on iris_read, so the first read after survey is always a cold miss.

**Solution:** After survey, pre-warm parent sections of returned claim-level results into the prefetch cache. Reuse the existing PrefetchEngine.insert API and structural prefetch strategy. No new subsystems needed.

### Tasks

- [x] Extract parent section ID from claim content_id (strip the :cN suffix) — add a helper fn parent_section_id(claim_content_id: &str) -> Option&lt;&str&gt; in types.rs
- [x] After survey returns results, collect unique parent section IDs from all claim-level results, fetch their SectionRecords from storage, and insert into PrefetchEngine with PrefetchStrategy::Structural
- [x] Skip pre-warming for sections already in the prefetch cache (PrefetchEngine.peek) or already delivered (session.is_delivered) — avoid redundant work
- [x] Add a new PrefetchStrategy::SurveyExpand variant to track hit rates separately from structural/sequential/topical strategies
- [x] Unit test: survey returning 3 claim hits from 2 different sections pre-warms exactly 2 parent sections, skips already-cached ones
- [x] E2E test: survey then iris_read of a parent section hits the prefetch cache (verify via prefetch_metrics.hits increasing)

---

## Phase I7: Expanded Corpus Support ✦ "Index the docs that actually help agents reason"

**Problem:** Iris currently only indexes the mdBook docs — the DESIGN.md (633 lines of architecture rationale) and CHANGELOG.md are not included. The most useful content for an agent is *why* decisions were made, not the code itself.

**Solution:** Accept multiple corpus paths or a list of glob patterns in config. Index DESIGN.md, CHANGELOG.md, and any additional markdown alongside the docs/ tree. Reuse the existing recursive discover_files + incremental ingestion pipeline.

### Tasks

- [x] Extend IrisConfig with a corpus_paths: Vec&lt;PathBuf&gt; field alongside the existing single corpus path — backwards compatible, single path becomes a vec of one
- [x] Support glob patterns in corpus_paths (e.g. "*.md", "docs/**") — resolve globs at startup using the existing discover_files recursive walker, deduplicate results
- [x] Update CLI --corpus flag to accept comma-separated paths or repeated flags: iris --corpus ./docs --corpus ./DESIGN.md --corpus ./CHANGELOG.md
- [x] Wire multi-path ingestion in main.rs: iterate corpus_paths, call ingest_directory_with_embeddings for directories and ingest_file_with_embeddings for individual files
- [x] Update .mcp.json default config to index ["./docs", "./DESIGN.md", "./CHANGELOG.md"] for the iris-rs project
- [x] Unit test: discover_files with mixed dirs and individual files returns correct combined list without duplicates
- [x] E2E test: start iris with multi-path corpus, verify iris_toc shows documents from all paths and iris_survey finds content across sources

---

## Phase W1: HTTP Foundation ✦ "Async HTTP client for fetching web documentation"

**Problem:** iris cannot access documentation hosted on the web — agents must leave iris to fetch external docs, losing context continuity

**Solution:** Add reqwest-based async HTTP client with timeout, retries, and User-Agent handling as the foundation for all web fetching

### Tasks

- [x] Add `reqwest` (with rustls-tls feature) to workspace dependencies and iris-core Cargo.toml
- [x] Create `web` module in iris-core with async `HttpClient` wrapper — configurable timeout (default 30s), retry count (default 2), and User-Agent header identifying iris
- [x] Implement URL normalization: resolve relative URLs, strip fragments, normalize trailing slashes, validate scheme (http/https only)
- [x] Unit tests for HttpClient: successful fetch, timeout handling, retry on 5xx, URL normalization edge cases

---

## Phase W2: HTML to Markdown ✦ "Clean markdown extraction from web pages"

**Problem:** Raw HTML is noisy and token-wasteful — nav bars, footers, ads, and scripts pollute the content

**Solution:** Build readability-style content extraction that identifies main content and converts to clean markdown, reusing the existing scraper crate

### Tasks

- [x] Build HTML-to-markdown converter using existing `scraper` crate — handle headings (h1-h6), paragraphs, code blocks (pre/code), ordered/unordered lists, tables, links, and images (as alt text)
- [x] Implement readability-style main content extraction: score DOM nodes by text density, strip nav/header/footer/sidebar/script/style elements, identify the `<main>` or `<article>` content container
- [x] Unit tests for HTML→markdown: convert sample documentation pages (with nav bars, sidebars, code samples) to markdown, verify clean output preserves headings, code blocks, and content structure

---

## Phase W3: llms.txt Support ✦ "First-class support for the llms.txt documentation standard"

**Problem:** Many documentation sites (844K+) now publish llms.txt/llms-full.txt — pre-processed, curated content optimized for LLMs that iris should prefer over raw HTML crawling

**Solution:** Auto-detect and fetch llms.txt/llms-full.txt from domain roots, parse the markdown link list format, and use pre-processed content when available

### Tasks

- [x] Implement llms.txt fetcher: given a domain, try GET `https://{domain}/llms-full.txt` then `https://{domain}/llms.txt` — return content if found (200 OK with text/plain or text/markdown)
- [x] Parse llms.txt markdown format: extract the H1 title, description blockquote, and categorized link lists (## sections with `- [title](url): description` entries)
- [x] Unit tests: parse sample llms.txt files (Anthropic, Cursor style), verify title/description/link extraction; verify llms-full.txt is returned as raw markdown content

---

## Phase W4: Web Fetch Pipeline ✦ "Smart orchestration: llms.txt → sitemap → single page fetch"

**Problem:** Different documentation sites require different fetching strategies — agents shouldn't need to know which one to use

**Solution:** Build WebFetcher that auto-selects the best strategy (llms.txt first, then sitemap, then direct fetch), converts to markdown, and feeds into the existing ingestion pipeline

### Tasks

- [x] Build `WebFetcher` orchestrator that auto-selects strategy: try llms-full.txt → llms.txt link list → sitemap.xml → direct page fetch, returning clean markdown for each discovered page
- [x] Pipe WebFetcher output into existing IngestionPipeline — fetched markdown goes through section extraction, claim extraction, summarization, and embedding generation unchanged
- [x] Store fetched web content in `~/.iris/web/<url-hash>/` with metadata file (source URL, fetch timestamp, ETag, content hash, page count)
- [x] Integration test: fetch a URL via WebFetcher, verify sections and claims appear in storage and are searchable via QueryService

---

## Phase W5: iris_fetch MCP Tool ✦ "One tool to fetch any web documentation into the corpus"

**Problem:** Agents need a simple, single-command way to say 'index this documentation site' without managing fetch strategies or ingestion details

**Solution:** New iris_fetch MCP tool that accepts a URL and optional crawl parameters, fetches and indexes content, and returns ingestion stats — content becomes immediately searchable via iris_survey

### Tasks

- [x] New MCP tool `iris_fetch(url, depth?, max_pages?, path_filter?)` in iris-mcp — single page mode (default) fetches one URL; crawl mode follows same-domain links up to depth/max_pages limits
- [x] iris_fetch returns structured response: pages_fetched, sections_indexed, claims_extracted, tokens_added, strategy_used (llms_txt/sitemap/crawl/single), plus budget_status
- [x] E2E test: call iris_fetch on a documentation URL, then verify iris_survey finds the fetched content and iris_toc lists the new documents

---

## Phase W6: Sitemap Crawling ✦ "Structured URL discovery via sitemap.xml"

**Problem:** Documentation sites with hundreds of pages need structured discovery — blindly following links is slow and may miss content

**Solution:** Parse sitemap.xml and sitemap index files for URL discovery, filter by path prefix, and fetch in parallel with rate limiting

### Tasks

- [x] Implement sitemap.xml parser: handle both `<urlset>` (flat) and `<sitemapindex>` (nested sitemap files), extract `<loc>` URLs with optional `<lastmod>` timestamps
- [x] Add path prefix filtering for sitemap URLs (e.g. only fetch URLs under `/docs/`) and configurable max page limit
- [x] Parallel page fetching with configurable concurrency (default 4 concurrent requests) and polite rate limiting (default 200ms between requests to same domain)
- [x] Unit tests: parse sample sitemap.xml and sitemap index files, verify URL extraction, path filtering, and lastmod parsing

---

## Phase W7: Web Cache and Refresh ✦ "Persistent caching with staleness detection for web docs"

**Problem:** Re-fetching unchanged documentation wastes time and bandwidth — but stale cached docs are worse than no docs

**Solution:** Cache fetched web content with ETags and timestamps, detect staleness via HTTP HEAD, and provide iris_refresh tool for on-demand or automatic updates

### Tasks

- [x] Track fetch metadata per URL in SQLite: source_url, fetch_timestamp, etag, last_modified, content_hash — new `web_cache` table with migration
- [x] Implement staleness detection: HTTP HEAD with If-None-Match (ETag) and If-Modified-Since headers — skip re-fetch if 304 Not Modified
- [x] New MCP tool `iris_refresh(url?)` — check all cached web sources (or a specific URL) for staleness, re-fetch and re-index changed content, report what was updated
- [x] Unit tests: staleness detection with mock HTTP responses (304, 200 with new ETag, timeout), cache expiry after configurable TTL

---

## Phase R1: Git Clone Integration ✦ "Fast shallow sparse clones for remote repository fetching"

**Problem:** Agents cannot access external codebases without manual cloning — and full clones are slow and wasteful

**Solution:** Use git's shallow sparse clone (--depth 1 --filter=blob:none --sparse) for 98% faster clones, targeting only needed directories

### Tasks

- [x] Build `GitFetcher` that shells out to git via `tokio::process::Command` — `git clone --depth 1 --filter=blob:none --sparse` into `~/.iris/remote/<repo-hash>/`
- [x] Implement sparse checkout support: after clone, run `git sparse-checkout set <paths>` to check out only requested directories/files
- [x] Track clone metadata: repo URL, branch, commit SHA, clone timestamp, checked-out paths — stored as TOML in the clone directory
- [x] Unit test: clone a small public repo (e.g. a test fixture repo), verify expected files are present and metadata is written

---

## Phase R2: iris_clone MCP Tool ✦ "One command to clone, index, and search any repository"

**Problem:** Agents need to quickly pull down reference implementations, library source code, or upstream dependencies and make them searchable

**Solution:** New iris_clone tool that accepts a repo URL with optional path/branch filters, clones via shallow sparse checkout, runs the ingestion pipeline, and caches results

### Tasks

- [x] New MCP tool `iris_clone(repo, paths?, branch?)` in iris-mcp — clone via GitFetcher, then run ingestion pipeline on checked-out content
- [x] iris_clone returns structured response: files_discovered, files_indexed, sections_extracted, clone_time_ms, index_time_ms, plus budget_status
- [x] Skip re-clone if repo is already cached and commit SHA matches remote HEAD — reuse existing index
- [x] E2E test: iris_clone a public repo, verify iris_toc shows its documents and iris_survey finds content from the cloned repo

---

## Phase R3: Unified Corpus URL Schemes ✦ "Mix local paths, web URLs, and repo URLs in corpus_paths"

**Problem:** Currently corpus_paths only accepts local filesystem paths — agents cannot declaratively configure remote sources

**Solution:** Extend corpus_paths to recognize https:// (web docs) and github:// (repos) schemes, routing each to the appropriate fetcher automatically

### Tasks

- [x] Extend corpus_paths URL parsing to recognize schemes: `https://` routes to WebFetcher, `github://owner/repo` or bare git URLs route to GitFetcher, plain paths stay as local filesystem
- [x] Update startup ingestion in main.rs to iterate corpus_paths, dispatch each to the appropriate fetcher, and merge all results into a unified corpus
- [x] Unit test: URL scheme parsing correctly classifies local paths, https URLs, and github:// URLs; integration test with mixed corpus_paths config

---

## Phase R4: Remote Source Staleness ✦ "Detect and refresh stale remote sources automatically"

**Problem:** Cloned repos go stale as upstream commits — agents need to know when their cached source is outdated

**Solution:** Track git remote HEAD SHA per clone, compare via git ls-remote on refresh, and unify with web staleness into a single iris_refresh mechanism

### Tasks

- [x] Implement git remote staleness check: `git ls-remote <repo> HEAD` to get current remote SHA, compare with cached clone SHA
- [x] Unify web and git staleness into `iris_refresh` — single tool checks both web cache ETags and git remote HEADs, re-fetches/re-clones as needed
- [x] Unit test: detect remote HEAD change via mock git ls-remote output; integration test: modify a cloned repo's remote, verify refresh detects the change

---

## Phase R5: Remote Pipeline Tests ✦ "End-to-end validation of the remote fetching pipeline"

**Problem:** Remote fetching involves network I/O, git subprocesses, and cross-pipeline integration — needs thorough testing

**Solution:** Integration and E2E tests covering clone+ingest+survey flow, unified search across local and remote sources, and error handling for auth failures and missing repos

### Tasks

- [x] Integration test: clone a repo, ingest its docs, verify iris_survey returns relevant content from the cloned source
- [x] E2E test: iris_clone + iris_fetch in the same session, verify iris_survey returns unified results from both local, web, and cloned sources
- [x] Error handling tests: nonexistent repo URL returns user-friendly error, private repo without auth returns clear auth failure message, empty repo is handled gracefully

---

## Phase C1: Tree-sitter Foundation ✦ "AST parsing infrastructure for structural code understanding"

**Problem:** iris treats code as plain text — it cannot understand function boundaries, type hierarchies, or module structure

**Solution:** Integrate tree-sitter with Rust grammar support to parse source code into ASTs, enabling structural code analysis

### Tasks

- [x] Add `tree-sitter` (0.25+) and `tree-sitter-rust` to workspace dependencies and iris-core Cargo.toml
- [x] Create `code` module in iris-core with `AstParser` struct — initializes tree-sitter parser with Rust language grammar, parses source bytes into a tree
- [x] Implement AST tree walker that visits top-level nodes and identifies item kinds: function_item, struct_item, enum_item, trait_item, impl_item, mod_item, type_item, const_item, static_item
- [x] Unit test: parse iris-core's own `config.rs` and `ingestion.rs`, verify correct AST node types are identified for structs, functions, and impls

---

## Phase C2: Symbol Extraction ✦ "Extract functions, structs, traits, and their metadata from ASTs"

**Problem:** Agents need to find specific code symbols by name, kind, or visibility — text search is imprecise and noisy

**Solution:** Walk tree-sitter ASTs to extract typed symbols with metadata: name, kind, visibility, signature, doc comments, byte range, parent scope

### Tasks

- [x] Define `Symbol` type with fields: name, kind (Function/Struct/Enum/Trait/Impl/Module/Const/TypeAlias), visibility (Public/PubCrate/Private), signature (first line), doc_comment, file_path, byte_range, module_path
- [x] Extract symbol metadata from Rust AST nodes: parse visibility modifiers, capture `///` doc comments from preceding comment nodes, build signature from the declaration line (without body)
- [x] Build `SymbolTable` collection type with query methods: find_by_name(pattern), filter_by_kind(kind), filter_by_visibility(vis), filter_by_module(path)
- [x] Unit test: extract symbols from iris-core source files, verify struct names, function signatures, visibility, and doc comments are captured correctly

---

## Phase C3: AST-Aware Code Chunking ✦ "Chunk code at function/struct boundaries, not arbitrary line counts"

**Problem:** Naive text chunking splits functions mid-body and merges unrelated code — destroying semantic coherence for embeddings

**Solution:** New ParserKind::Code that uses tree-sitter to split at AST boundaries, producing multi-resolution chunks: file summaries, symbol stubs (signature + doc), and full implementations

### Tasks

- [x] Add `ParserKind::Code` variant — detected for `.rs`, `.ts`, `.js`, `.py`, `.go`, `.java`, `.c`, `.cpp`, `.h` extensions
- [x] Implement AST-aware code chunker: split source files at function/struct/enum/trait/impl boundaries, producing one Section per top-level symbol with correct byte ranges
- [x] Multi-resolution code chunks: file-level section (module doc + public symbol list as summary), symbol stubs (signature + doc comment, no body), full symbol (complete source including body)
- [x] Section IDs for code follow pattern: `file.rs#module_path::SymbolName` (e.g. `config.rs#config::IrisConfig`, `ingestion.rs#ingestion::IngestionPipeline::ingest_directory`)
- [x] Unit test: chunk a Rust source file, verify chunk boundaries align with AST node boundaries — no function split mid-body, no struct split from its impl block

---

## Phase C4: Symbol Storage Schema ✦ "SQLite-backed symbol index with relationship tracking"

**Problem:** Symbol metadata and relationships (caller→callee, trait→impl, imports) need persistent, queryable storage

**Solution:** New SQLite tables for symbols and symbol_refs, with storage trait extensions for CRUD operations and relationship queries

### Tasks

- [x] New SQLite table `symbols`: id, file_path, name, kind, visibility, signature, doc_comment, module_path, line_start, line_end — with migration
- [x] New SQLite table `symbol_refs`: from_symbol_id, to_symbol_id, ref_kind (Calls/Implements/Imports/Uses) — with migration
- [x] Extend Storage trait with symbol CRUD: insert_symbols, list_symbols(filters), get_symbol(id), insert_symbol_refs, query_refs(symbol_id, ref_kind?)
- [x] Unit test: insert and query symbols and relationships, verify filtering by kind/visibility/module works correctly

---

## Phase C5: Code Intelligence MCP Tools ✦ "iris_symbols, iris_definition, iris_references — structural code navigation"

**Problem:** Agents need to navigate code structurally: find a function by name, jump to its definition, find all callers

**Solution:** Three new MCP tools: iris_symbols (search/browse symbol index), iris_definition (jump to source), iris_references (find callers/implementors/importers)

### Tasks

- [x] New MCP tool `iris_symbols(query?, kind?, module?, visibility?)` — search the symbol index with fuzzy name matching and exact kind/module/visibility filters, returns symbol list with file, line, signature, doc preview
- [x] New MCP tool `iris_definition(symbol_id)` — returns full source code of the symbol with 3 lines of surrounding context, heading path showing module hierarchy, and budget tracking
- [x] New MCP tool `iris_references(symbol_id, ref_kind?)` — returns all references: callers (Calls), implementors (Implements), importers (Imports), with source locations
- [x] E2E test: index a codebase with code intelligence, verify iris_symbols finds expected functions/structs, iris_definition returns correct source, iris_references finds callers

---

## Phase C6: Unified Code + Doc Search ✦ "iris_survey searches code symbols alongside documentation"

**Problem:** Code and documentation live in separate search silos — agents must know which to search

**Solution:** Embed code symbols in the same HNSW index as document sections, extending iris_survey to return both code and doc results ranked together

### Tasks

- [ ] Embed code symbol stubs (signature + doc comment) into the same HNSW vector index used for document sections — new VectorId variants for code symbols
- [ ] Extend iris_survey to return both document sections AND code symbols in a unified ranked result list — add `resolution: "symbol_stub"` and `resolution: "symbol_full"` variants
- [ ] E2E test: index a project with both docs and code, iris_survey with a query like "how does ingestion work" returns both documentation sections and relevant code symbols

---

## Phase C7: Multi-Language Support ✦ "Tree-sitter grammars for TypeScript, Python, Go, and more"

**Problem:** Code intelligence limited to Rust misses the majority of codebases agents work with

**Solution:** Add tree-sitter grammars for TypeScript/JavaScript, Python, and Go with unified symbol extraction interface and language-specific AST walkers

### Tasks

- [ ] Build a dynamic grammar loader that downloads and caches tree-sitter grammar `.so`/`.dylib` files on demand — when iris encounters a new file extension, it fetches the matching grammar from the tree-sitter grammar registry automatically
- [ ] Maintain a language registry mapping file extensions to tree-sitter grammar crate names — cover all 30+ mainstream languages (Rust, TypeScript, JavaScript, Python, Go, Java, C, C++, C#, Ruby, Swift, Kotlin, Scala, PHP, Elixir, Haskell, Lua, Zig, OCaml, Dart, R, Shell/Bash, SQL, HTML, CSS, YAML, TOML, JSON, Terraform/HCL, Dockerfile, Protobuf, etc.)
- [ ] Implement a generic AST symbol extractor with language-agnostic heuristics: identify nodes with names (functions, classes, types) by tree-sitter node kind patterns common across grammars (e.g. `*_definition`, `*_declaration`, `function_*`, `class_*`)
- [ ] Add language-specific AST walker refinements for high-value languages: Rust (traits, impls, derive macros), TypeScript (interfaces, type aliases, decorators), Python (classes, decorators, type hints), Go (interfaces, methods, receivers)
- [ ] Automatic language detection from file extension in `detect_parser_kind` — route to tree-sitter grammar if available, fall back to text-based parsing for unknown extensions, never fail on an unsupported language
- [ ] Unit tests: parse and extract symbols from at least 5 languages (Rust, TypeScript, Python, Go, Java), verify the generic extractor produces reasonable symbols even for languages without a specific refinement

