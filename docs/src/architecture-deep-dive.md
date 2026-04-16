# iris-rs Architecture: The Deep Dive

> **iris** is a Rust-native MCP server that manages LLM agent context windows
> like a CPU cache controller — with session tracking, predictive prefetching,
> budget management, and coherence.

---

## The Big Picture

Think of iris as a **smart librarian** that sits between an LLM agent and a codebase.
Instead of the agent naively reading files and losing track of what it's already seen,
iris indexes everything, tracks what's been delivered, predicts what's needed next,
and manages the agent's finite context window like a CPU manages its L1/L2 cache.

```d2
direction: down

agent: LLM Agent (Claude, etc.) {
  call1: iris_survey('authentication')
  call2: iris_read('src/auth.rs#login')
  call3: iris_symbols(kind='struct', query='Session')
}

mcp: iris-mcp (Transport Layer) {
  server: IrisServer — tool routing, JSON-RPC
  sessions: SessionRegistry
  prefetch: PrefetchEngine
  analytics: Cross-session analytics

  server -> sessions
  server -> prefetch
  server -> analytics
}

core: iris-core (Domain Logic) {
  query: QueryService — survey, read, extract, symbols, refs
  search: MultiResolutionSearch\ndense + sparse + rerank
  ingest: IngestionPipeline\nparse → extract → embed
  coherence: CoherenceEngine\nfile watch → re-index
  session: Session shadow\ndedup, delta, budget

  infra: {
    embed: Embedding (ONNX) {
      shape: rectangle
    }
    storage: Storage (SQLite) {
      shape: cylinder
    }
    index: Index (HNSW) {
      shape: cylinder
    }
    parser: Parser (tree-sitter) {
      shape: rectangle
    }
  }

  query -> search
  search -> infra.embed
  search -> infra.index
  ingest -> infra.parser
  ingest -> infra.storage
}

agent -> mcp: MCP Protocol\n(stdio or HTTP)
mcp -> core
```

---

## Workspace Structure

```
iris-rs/
├── iris-cli/              ← Binary entry point, CLI commands
│   └── src/
│       ├── main.rs        ← CLI parsing, subcommand dispatch
│       ├── commands/      ← serve, index, init, search, export/import, hooks
│       ├── infra.rs       ← Storage, embedder, index bootstrap
│       ├── ingestion.rs   ← Corpus ingestion orchestration
│       ├── instance.rs    ← Single-instance lock, stdio↔HTTP proxy
│       └── proxy.rs       ← Secondary instance proxy over HTTP
│
├── iris-mcp/              ← MCP server adapter (depends on iris-core + rmcp)
│   └── src/
│       ├── server/        ← IrisServer: tool handlers, session management
│       ├── auth.rs        ← OAuth for cloud deployments
│       ├── proxy.rs       ← Thin proxy that delegates to iris-daemon
│       └── error.rs       ← MCP-specific error types
│
├── iris-daemon/           ← HTTP API over Unix domain socket
│   └── src/
│       ├── daemon.rs      ← Axum server, lifecycle management
│       ├── registry.rs    ← CorpusRegistry: manage multiple corpora
│       ├── ask.rs         ← Query handlers
│       ├── inference.rs   ← Embedding service
│       └── state.rs       ← Shared daemon state
│
├── iris-api/              ← Shared wire types (no iris-core dependency)
│   └── src/
│       ├── query.rs       ← Request/response types
│       └── client.rs      ← DaemonClient for UDS communication
│
├── iris-core/             ← Pure domain logic, NO transport dependencies
│   └── src/
│       ├── service/       ← QueryService: the main API facade
│       ├── ingestion/     ← File discovery → parse → embed pipeline
│       ├── coherence.rs   ← File watcher → incremental re-index
│       ├── session/       ← The "cache controller" brain
│       │   ├── types.rs   ← Session shadow (delivered content tracker)
│       │   ├── budget.rs  ← Token budget & pressure levels
│       │   ├── prefetch/  ← Predictive pre-warming engine
│       │   ├── window.rs  ← Context window estimator
│       │   ├── delta.rs   ← Content change deltas
│       │   └── registry.rs ← Multi-session management
│       ├── embedding/     ← ONNX + optional Candle Metal GPU
│       ├── index/         ← HNSW + inverted index (SPLADE)
│       ├── storage/       ← SQLite persistence layer
│       ├── parser/        ← Markdown, HTML, PDF parsers
│       ├── code/          ← tree-sitter, symbols, cross-lang bridges
│       ├── extraction/    ← Claims, relationships, summaries
│       └── ...
│
└── iris-app/src-tauri/    ← Tauri v2 desktop app with system tray
    └── src/
        ├── main.rs        ← Tauri setup, tray menu
        ├── commands.rs    ← Frontend-facing commands
        └── state.rs       ← App state shared with daemon
```

### Dependency Rule

```
iris-cli  →  iris-mcp  →  iris-core
              ↓              ↑
          iris-api     NO transport deps
              ↑        (pure domain logic)
          iris-daemon
              ↓
          iris-core

iris-app  →  iris-daemon  →  iris-core
```

`iris-core` **never** imports MCP types. `iris-api` never depends on `iris-core`. The boundaries are enforced structurally.

---

## How It Boots Up

When you run `iris serve`, here's the startup sequence:

```
main()
  │
  ├─ 1. Parse CLI args (clap)
  │
  ├─ 2. Load config (.iris.toml + ~/.iris/config.toml)
  │
  ├─ 3. acquire_role()
  │     ├─ Try to grab a file lock on ~/.iris/corpora/<hash>/iris.lock
  │     ├─ Got it? → You're the PRIMARY (runs the real server)
  │     └─ Locked? → You're a SECONDARY (proxies stdio→HTTP to primary)
  │
  ├─ 4. init_infrastructure()
  │     ├─ Open/create SQLite database
  │     ├─ Load FastEmbedder (ONNX model: all-MiniLM-L6-v2)
  │     │     └─ With CoreML execution provider on Apple Silicon
  │     └─ Load/create HnswIndex (384-dim, cosine similarity)
  │
  ├─ 5. build_server()
  │     ├─ Create QueryService(storage, embedder, index)
  │     ├─ Create IrisServer(service, registry, prefetch, ...)
  │     ├─ Enable web fetcher (for iris_fetch)
  │     ├─ Enable git fetcher (for iris_clone)
  │     └─ Spawn coherence file watcher
  │
  └─ 6. Start transport
        ├─ stdio: MCP over stdin/stdout (default for Claude Code)
        └─ http: Streamable HTTP MCP server (for cloud)
```

### Single-Instance Protocol

iris ensures only one server runs per repo. The trick is clever:

```
instance.rs:

  1. Hash the corpus paths → deterministic port in 49152–65535
  2. Try to acquire exclusive file lock
  3. If locked → read the port file → proxy stdio↔HTTP to primary
  4. If unlocked → you're primary → also spawn HTTP listener on that port
```

This means multiple Claude Code sessions on the same repo share one index.

---

## The Ingestion Pipeline

Before iris can answer queries, it needs to index the codebase. Here's the flow:

```
                         IngestionPipeline
                               │
         ┌─────────────────────┼─────────────────────┐
         │                     │                      │
    File Discovery        Parse & Split          Embed & Store
         │                     │                      │
    Walk directory        For each file:         For each section:
    Filter by extension   │                      │
    Hash for incremental  ├─ Detect parser       ├─ Embed text → Vec<f32>
    Skip unchanged files  │  (md/html/pdf/code)  ├─ Insert into HNSW index
                          │                      ├─ Extract claims
                          ├─ Parse into sections ├─ Detect relationships
                          │  with headings       └─ Store in SQLite
                          │
                          └─ For code files:
                             ├─ tree-sitter AST parse
                             ├─ Extract symbols (structs, fns, traits...)
                             ├─ Extract references (calls, imports)
                             └─ Detect cross-language bridges
```

### What Gets Stored

```
SQLite Database (~/.iris/corpora/<hash>/content.db)
  │
  ├── documents        ─ file path, hash, root kind
  ├── sections         ─ heading path, text, token count, parent
  ├── claims           ─ atomic assertions extracted from sections
  ├── relationships    ─ claim-to-claim connections
  ├── symbols          ─ name, kind, visibility, module, signature, source
  ├── symbol_refs      ─ caller→callee, importer→importee, etc.
  ├── bridge_endpoints ─ cross-language binding sites
  ├── bridge_links     ─ matched export↔import pairs
  ├── embedding_cache  ─ precomputed vectors (keyed by content hash)
  ├── corpus_roots     ─ tracked source directories
  ├── web_sources      ─ fetched URL metadata (ETag, last-modified)
  ├── section_accesses ─ cross-session access frequency
  └── co_accesses      ─ which sections are accessed together
```

### The Embedding Stack

```
Text → FastEmbedder (all-MiniLM-L6-v2, ONNX Runtime)
           │
           ├─ Dense vector: 384-dim float32
           │     └─ Stored in HNSW index for ANN search
           │
           └─ Optional: SPLADE sparse embedding
                 └─ Stored in inverted index for keyword-aware search

Query time:
  Dense results ─┐
                 ├─ RRF fusion ─→ Candidates ─→ Cross-encoder rerank ─→ Final results
  Sparse results ┘
```

The embeddings are cached in SQLite keyed by content hash — if the text hasn't
changed, the embedding is reused without re-running ONNX inference.

---

## The Query Path: What Happens When You Call `iris_survey`

This is the most important flow. Let's trace it end-to-end:

```d2
shape: sequence_diagram

agent: Agent
server: IrisServer
prefetch: PrefetchEngine
sessions: SessionRegistry
query: QueryService
search: MultiResSearch
storage: SQLite + HNSW

agent -> server: iris_survey("authentication middleware")
server -> prefetch: "1. check warm cache"
server -> sessions: "2. get_or_create(session_id)"
server -> query: "3. survey_excluding(query, top_k, delivered_ids)"
query -> search: "4. multi-resolution search"
search -> search: "embed → HNSW kNN → optional SPLADE\n→ RRF fusion → optional rerank"
search -> storage: "5. resolve content"
storage -> query: "Vec<SurveyResult>"
query -> server: "results"
server -> sessions: "7. record delivery + budget + analytics"
server -> prefetch: "9. pre-warm predicted next reads"
server -> agent: "10. JSON response + budget_status"
```

### Key Insight: Deduplication

The `survey_excluding` call filters out section IDs that the session has already
delivered. This prevents the agent from getting the same content twice. The session
shadow tracks everything:

```
Session {
    delivered: BTreeMap<ContentId, DeliveredItem>,  // what's been sent
    trajectory: Vec<ContentId>,                      // access order
    stale: HashSet<ContentId>,                       // invalidated by file changes
}
```

---

## The Session Shadow: iris's "Cache Controller" Brain

This is the most novel subsystem. It models the agent's context window **from the
outside**, predicting what the agent has retained and what's been evicted.

### The Window Estimator

```
WindowEstimator {
    capacity: 100_000 tokens,
    policy: FIFO,
    entries: VecDeque<(content_id, token_count)>,
    current_tokens: 47_320,
    evicted: ["old-section-1", "old-section-2", ...]
}

When new content is delivered:
  1. Add entry to back of queue
  2. current_tokens += new_tokens
  3. While current_tokens > capacity:
     │  Pop from front (FIFO)
     │  current_tokens -= popped.tokens
     └  Move to evicted list
```

### Budget Pressure Levels

The budget tracker maps window utilization to three pressure levels:

```
 0%                    80%              95%            100%
  ├─────── Normal ──────┼── Elevated ────┼── Critical ──┤
  │                     │                │              │
  │ Full section text   │ Compressed to  │ Summaries    │
  │ at requested        │ claim-level,   │ only, strong │
  │ resolution          │ eviction recs  │ eviction recs│
```

When pressure is `Elevated`, iris automatically compresses responses to claim-level
granularity. At `Critical`, it returns only summaries and strongly recommends the
agent evict old content.

This is included in **every response** as `budget_status`:

```json
{
  "budget_status": {
    "pressure_level": "normal",
    "tokens_used": 4112,
    "tokens_remaining": 95888,
    "utilization": 0.041
  }
}
```

---

## The Prefetch Engine: Predicting What's Needed Next

After each `iris_read`, the prefetch engine speculatively pre-warms sections the
agent is likely to request next. It uses five strategies:

```d2
direction: right

read: "Agent reads\nsection N" {
  shape: rectangle
}

engine: PrefetchEngine {
  seq: "1. Sequential\n(cache line prefetch)" {
    tooltip: Pre-warm section N+1, N+2 in same document
  }
  topical: "2. Topical\n(branch prediction)" {
    tooltip: Query HNSW with running EMA topic vector
  }
  structural: "3. Structural\n(spatial locality)" {
    tooltip: Pre-warm sibling sections at same depth
  }
  cross: "4. Cross-Session\n(shared cache)" {
    tooltip: Pre-warm frequently co-accessed sections
  }
  survey: "5. Survey Expand\n(TLB prefetch)" {
    tooltip: Pre-warm parent sections of claim hits
  }
}

cache: "Prefetch cache\n(LRU, capacity=50)" {
  shape: cylinder
}

read -> engine
engine.seq -> cache
engine.topical -> cache
engine.structural -> cache
engine.cross -> cache
engine.survey -> cache
```

### The Topic Tracker

The topical prefetch strategy maintains a running "topic vector" using
exponential moving average (EMA):

```
topic_vector = alpha * latest_embedding + (1 - alpha) * topic_vector

  alpha = 0.3 (configurable)

  Early in session: topic drifts quickly as agent explores
  Later: topic stabilizes, prefetch becomes more accurate
```

### Cache Structure

```
PrefetchCache (LRU, capacity=50)
  │
  ├── HashMap<content_id, CacheEntry>   ← O(1) lookup
  └── VecDeque<content_id>              ← LRU ordering

  Metrics tracked per strategy:
    Sequential:    hits=12, misses=3   (80% hit rate)
    Topical:       hits=8,  misses=7   (53% hit rate)
    Structural:    hits=5,  misses=2   (71% hit rate)
    CrossSession:  hits=3,  misses=1   (75% hit rate)
```

---

## The Coherence Engine: Staying in Sync

When files change on disk, iris needs to update the index AND notify active sessions.

```d2
direction: right

fs: File System {
  shape: rectangle
}

watcher: FileWatcher\n(notify crate) {
  shape: rectangle
}

engine: CoherenceEngine {
  step1: "1. Re-parse file"
  step2: "2. Re-extract claims"
  step3: "3. Re-embed"
  step4: "4. Update SQLite"
  step5: "5. Update HNSW"

  step1 -> step2 -> step3 -> step4 -> step5
}

sessions: SessionRegistry {
  mark: Mark affected\nsections as stale
  queue: Queue coherence\nalerts for agent

  mark -> queue
}

fs -> watcher: notify
watcher -> engine: "CoherenceEvent\n(Created/Modified/Removed)"
engine -> sessions
```

When the agent next calls any iris tool, it receives pending alerts:

```json
{
  "coherence_alerts": [
    "Section 'src/auth.rs#login' has been modified since last delivery"
  ]
}
```

And `iris_read` on a stale section delivers only the **delta** (what changed),
not the full text again.

---

## Content Resolution: Multi-Resolution Delivery

iris indexes content at multiple granularity levels and delivers at the
appropriate resolution based on context budget pressure:

```
Resolution Levels:

  Document ──── "Here's the entire file"
       │
  Section ───── "Here's the #login function section"
       │
  Claim ──────── "login() validates JWT tokens and returns a User struct"
       │
  Summary ────── "Auth module: handles JWT validation, session management"

                 ▲                    ▲                    ▲
              Normal               Elevated             Critical
           budget pressure      budget pressure      budget pressure
```

### Delta Delivery

When the agent re-reads a section it already has, iris computes a diff:

```
ContentDelta {
    lines: [
        Unchanged("fn login(token: &str) -> Result<User> {"),
        Removed("    let claims = decode_jwt(token)?;"),
        Added("    let claims = verify_jwt(token, &config.secret)?;"),
        Unchanged("    Ok(User::from(claims))"),
    ],
    additions: 1,
    removals: 1,
}
```

This saves massive amounts of context window space — only the changes are delivered.

---

## Code Intelligence: Beyond Text Search

iris doesn't just search text. It builds a rich code model:

### Symbol Table

```
tree-sitter AST parse
       │
       ├─ Extract symbols: structs, functions, traits, enums, impls
       │    Name, kind, visibility, module path, signature, doc comments
       │
       ├─ Extract references: who calls what, who imports what
       │    Callers, callees, implementors, importers
       │
       └─ Language refinements: per-language post-processing
            Rust, TypeScript, Python, Go, Java, C, C++, Swift, Kotlin
```

### Cross-Language Bridges

iris can detect and link cross-language bindings:

```d2
direction: right

rust: Rust {
  napi: "#[napi]\nfn greet(s: String)" { shape: rectangle }
  pyo: "#[pyfunction]\nfn compute(x: f64)" { shape: rectangle }
  tauri: "#[tauri::command]\nfn open_file(path)" { shape: rectangle }
}

other: JavaScript / Python {
  js_native: "import { greet }\nfrom './native'" { shape: rectangle }
  py_import: "from mylib import\n    compute" { shape: rectangle }
  invoke: "invoke('open_file',\n   { path })" { shape: rectangle }
}

rust.napi -> other.js_native: napi { style.stroke-width: 3 }
rust.pyo -> other.py_import: pyo3 { style.stroke-width: 3 }
rust.tauri -> other.invoke: tauri { style.stroke-width: 3 }
```

Also: wasm-bindgen, HTTP routes (server↔client matching).

The `BridgeLinker` runs a two-pass pipeline:
1. **Extract** endpoints from all source files
2. **Link** export↔import pairs by binding key (exact match → case-normalized → semantic fallback)

---

## Storage Layer

All persistent state lives in SQLite, accessed through `SqliteStorage`:

```
SqliteStorage {
    conn: Arc<Mutex<Connection>>
}

Key design decisions:
  - Arc<Mutex> for sharing across spawn_blocking tasks
  - Mutex held only during blocking call, never across .await
  - WAL mode for concurrent readers
  - Connection pooling via single shared connection
  - All access through the Storage trait (async interface)
```

The `Storage` trait provides the async interface that `QueryService` depends on,
keeping the storage implementation swappable.

---

## The MCP Server: How Tools Map to Code

Each iris MCP tool maps to a method chain:

```
MCP Tool              IrisServer method        QueryService method
─────────────────────────────────────────────────────────────────
iris_survey      →    handle_survey()      →    survey_excluding()
iris_read        →    handle_read()        →    read_section()
iris_extract     →    handle_extract()     →    extract_claims()
iris_symbols     →    handle_symbols()     →    search_symbols()
iris_definition  →    handle_definition()  →    symbol_definition()
iris_references  →    handle_references()  →    symbol_references()
iris_toc         →    handle_toc()         →    table_of_contents()
iris_budget      →    handle_budget()      →    (session state only)
iris_compress    →    handle_compress()    →    compress_for_eviction()
iris_evicted     →    handle_evicted()     →    (session state only)
iris_fetch       →    handle_fetch()       →    WebFetcher + ingest
iris_clone       →    handle_clone()       →    GitFetcher + ingest
iris_refresh     →    handle_refresh()     →    WebFetcher staleness check
iris_bridge      →    handle_bridge()      →    search_bridge_links()
iris_related     →    handle_related()     →    related_claims()
iris_task        →    handle_task()        →    (task manager state)
```

Every response wraps the result data with `budget_status`, so the agent always
knows how much context budget it has remaining.

---

## Putting It All Together: A Typical Session

```
 Time   Agent Action                iris Response                Internal State
─────┬──────────────────────┬──────────────────────────────┬─────────────────────
  0  │ Session starts       │                              │ Session created,
     │                      │                              │ budget=100K tokens
     │                      │                              │
  1  │ iris_toc()           │ 207 documents, 3197 sections │ Trajectory: []
     │                      │                              │
  2  │ iris_survey(         │ Top 5 ranked results         │ Trajectory: [survey-1]
     │   "auth middleware") │ with scores and text         │ Delivered: 5 sections
     │                      │ budget: 3% used              │ Tokens used: 3,000
     │                      │                              │
     │                      │                              │ PREFETCH: pre-warm
     │                      │                              │ siblings of top hit
     │                      │                              │
  3  │ iris_read(           │ Full section text             │ Trajectory: [s, read-1]
     │   "src/auth.rs#      │ + heading path               │ Tokens used: 5,500
     │    login")           │ budget: 5.5% used             │
     │                      │                              │ PREFETCH: pre-warm
     │                      │                              │ auth.rs#logout (seq)
     │                      │                              │ auth.rs#validate (struct)
     │                      │                              │ session.rs#create (topic)
     │                      │                              │
  4  │ iris_read(           │ CACHE HIT! Instant delivery  │ Sequential prefetch
     │   "src/auth.rs#      │ (was pre-warmed)             │ paid off!
     │    logout")          │ budget: 7% used              │
     │                      │                              │
  5  │ iris_symbols(        │ Symbol list with signatures  │
     │   kind="struct",     │ and file locations           │
     │   query="User")     │                              │
     │                      │                              │
  6  │ iris_definition(     │ Full source code of          │
     │   "sym-...User")    │ User struct                  │
     │                      │                              │
  7  │ iris_references(     │ All callers, implementors    │
     │   "sym-...User")    │ of User                      │
     │                      │                              │
     │        ... (many more reads, agent is working) ...  │
     │                      │                              │
 50  │ iris_survey(         │ Results at CLAIM resolution  │ Tokens used: 82,000
     │   "error handling") │ (compressed — elevated       │ Pressure: ELEVATED
     │                      │  pressure detected)          │ Eviction recs attached
     │                      │ budget: 82% used             │
     │                      │ eviction_recommendations:    │
     │                      │   ["old-section-1", ...]     │
     │                      │                              │
 51  │ iris_compress(       │ Compressed summary for       │ Agent preparing to
     │   "old-section-1")  │ the section about to be      │ evict old content
     │                      │ evicted                      │
     │                      │                              │
 52  │ iris_evicted(        │ Acknowledged. Session shadow  │ Window estimator
     │   ["old-section-1"])│ updated.                     │ frees token budget
     │                      │ budget: 75% used             │
     │                      │                              │
     │        ... (file changes on disk) ...               │
     │                      │                              │
 60  │ iris_read(           │ DELTA delivery:              │ Coherence alert
     │   "src/auth.rs#      │ Only the changed lines       │ was queued, now
     │    login")           │ (3 lines changed)            │ delivered with read
     │                      │ + coherence_alert            │
```

---

## Key Design Decisions

### Why "Like a CPU Cache"?

| CPU Cache Concept    | iris Equivalent                                    |
|---------------------|----------------------------------------------------|
| Cache line           | Section (a heading-delimited chunk of content)      |
| L1/L2/L3 hierarchy  | Claim → Section → Document resolution levels        |
| Cache hit            | Content already in session → skip/delta delivery    |
| Cache miss           | Cold read → full retrieval from storage + embedding  |
| Prefetch             | Speculative pre-warming of predicted next reads      |
| Write-back           | Coherence engine: file changes → re-index + alerts   |
| Cache coherence      | Session stale marking across concurrent sessions     |
| Eviction (LRU/FIFO) | Window estimator evicts oldest delivered content     |
| Cache pressure       | Budget pressure levels (Normal/Elevated/Critical)    |

### Why Rust?

- ONNX inference is CPU-bound — zero-cost abstractions matter
- SQLite + HNSW are memory-mapped — Rust's ownership model prevents data races
- MCP server needs to be fast and low-memory for always-on background service
- `tree-sitter` bindings are native C — Rust FFI is zero-overhead
- `tokio` async runtime for concurrent I/O without threads per connection

### Why Local Embeddings (not API)?

- **Latency**: Local ONNX inference (~5ms/embed) vs API round-trip (~200ms)
- **Cost**: Zero marginal cost per embedding vs per-token API pricing
- **Privacy**: Code never leaves the machine
- **Offline**: Works without internet
- **CoreML**: Apple Neural Engine acceleration on Apple Silicon
