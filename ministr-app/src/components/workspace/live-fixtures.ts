/**
 * Rich fixtures for the LIVE workspace stories (Workspace/WorkspaceScreen).
 *
 * The composed `WorkspaceScreen` mounts the real shipped surfaces as facets,
 * each of which fetches its own data through `invoke(...)`. The Storybook
 * Tauri-mock (`withTauriMock`) intercepts those calls; this module is the
 * fixture bundle that makes every facet render POPULATED rather than its empty
 * state:
 *
 *   • Fleet / spine  ← LIVE_CORPORA + LIVE_STATUS (a multi-project constellation)
 *   • Activity       ← `list_sessions`  (a live board incl. a subagent lineage)
 *   • Explore        ← `list_corpus_files` + `read_file` + `file_occurrences`
 *                      + `search_symbols` + `symbol_definition`/`symbol_references`
 *                      (file tree + landing, and the click→open→neighborhood chain)
 *   • Tend           ← spine corpus + `list_supported_models`
 *   • Ask            ← `inference_health` + seeded localStorage threads/pinned
 *
 * Shapes mirror `lib/types.ts` + `surfaces/ask/{thread,internals}.ts` exactly.
 */
import type {
  BridgeLink,
  CorpusInfo,
  DaemonStatus,
  FileContent,
  FileInfo,
  Occurrence,
  SessionDetail,
  SymbolDefinitionDetail,
  SymbolInfo,
  SymbolRef,
} from "../../lib/types";
import type { TauriFixtures } from "../../../.storybook/tauri-mock";

// ── Projects — a real constellation, not one idle corpus. ──────────────────

function mkCorpus(
  over: Partial<CorpusInfo> & { id: string; paths: string[] },
): CorpusInfo {
  return {
    status: { state: "idle" },
    files_indexed: 0,
    sections_count: 0,
    embeddings_count: 0,
    active_sessions: 0,
    symbols_count: 0,
    ...over,
  };
}

const HOUR = 3_600_000;

export const LIVE_CORPORA: CorpusInfo[] = [
  mkCorpus({
    id: "ministr",
    display_name: "ministr",
    paths: ["/Users/alrik/Code/ministr"],
    files_indexed: 1284,
    sections_count: 9210,
    embeddings_count: 41233,
    symbols_count: 18422,
    active_sessions: 3,
    last_indexed: Date.now() - 0.4 * HOUR,
    model: "jina-code-v2",
  }),
  mkCorpus({
    id: "ministr-private",
    display_name: "ministr-private",
    paths: ["/Users/alrik/Code/ministr-private"],
    files_indexed: 312,
    sections_count: 2104,
    embeddings_count: 9920,
    symbols_count: 4210,
    active_sessions: 1,
    last_indexed: Date.now() - 5 * HOUR,
    model: "jina-code-v2",
  }),
  mkCorpus({
    id: "atlas-web",
    display_name: "atlas-web",
    paths: ["/Users/alrik/Code/atlas-web"],
    status: { state: "indexing", files_done: 740, files_total: 1120 },
    files_indexed: 1120,
    sections_count: 6402,
    embeddings_count: 18800,
    symbols_count: 9044,
    last_indexed: Date.now() - 26 * HOUR,
    model: "jina-code-v2",
  }),
  mkCorpus({
    id: "rig-engine",
    display_name: "rig-engine",
    paths: ["/Users/alrik/Code/rig-engine"],
    files_indexed: 2890,
    sections_count: 21044,
    embeddings_count: 88120,
    symbols_count: 41200,
    last_indexed: Date.now() - 3 * 24 * HOUR,
    model: "jina-code-v2",
  }),
  mkCorpus({
    id: "notes-vault",
    display_name: "notes-vault",
    paths: ["/Users/alrik/Notes"],
    warming: true,
    files_indexed: 540,
    sections_count: 3120,
    embeddings_count: 0,
    symbols_count: 0,
  }),
];

export const LIVE_STATUS: DaemonStatus = {
  version: "0.3.1",
  uptime_secs: 8460,
  memory_mb: 612,
  model: "jina-code-v2",
  model_dimension: 768,
  corpora: LIVE_CORPORA,
  total_sessions: 5,
  log_path: "/Users/alrik/Library/Logs/ministr/daemon.log",
};

// ── Activity — a live session board with a subagent lineage. ───────────────

function mkSession(
  over: Partial<SessionDetail> & { session_id: string; corpus_id: string },
): SessionDetail {
  const used = over.tokens_used ?? 40_000;
  const remaining = over.tokens_remaining ?? 160_000;
  return {
    current_turn: 12,
    delivered_count: 64,
    tokens_used: used,
    tokens_remaining: remaining,
    utilization: used / (used + remaining),
    pressure_level: "nominal",
    total_deliveries: 88,
    cumulative_tokens_delivered: 612_000,
    total_tokens_saved: 1_240_000,
    total_evictions: 7,
    total_compressions: 23,
    dedup_hits: 412,
    compression_ratio: 3.6,
    context_window_tokens: used + remaining,
    client_name: "claude-code",
    ...over,
  };
}

export const LIVE_SESSIONS: SessionDetail[] = [
  mkSession({
    session_id: "sess-arch-01",
    corpus_id: "ministr",
    client_name: "claude-code",
    current_turn: 41,
    tokens_used: 86_000,
    tokens_remaining: 114_000,
    pressure_level: "elevated",
    total_tokens_saved: 3_410_000,
    dedup_hits: 1_204,
  }),
  mkSession({
    session_id: "sess-arch-01-sub-a",
    corpus_id: "ministr",
    parent_session_id: "sess-arch-01",
    client_name: "claude-code · Task",
    current_turn: 9,
    tokens_used: 22_000,
    tokens_remaining: 178_000,
    pressure_level: "nominal",
    total_tokens_saved: 240_000,
    dedup_hits: 88,
  }),
  mkSession({
    session_id: "sess-review-02",
    corpus_id: "ministr",
    client_name: "cursor",
    current_turn: 27,
    tokens_used: 142_000,
    tokens_remaining: 58_000,
    pressure_level: "high",
    total_tokens_saved: 2_010_000,
    dedup_hits: 760,
  }),
  mkSession({
    session_id: "sess-hotfix-03",
    corpus_id: "ministr",
    client_name: "claude-code",
    current_turn: 64,
    tokens_used: 187_000,
    tokens_remaining: 13_000,
    pressure_level: "critical",
    total_tokens_saved: 4_920_000,
    total_evictions: 31,
    dedup_hits: 1_880,
  }),
  mkSession({
    session_id: "sess-cloud-04",
    corpus_id: "ministr-private",
    client_name: "windsurf",
    current_turn: 15,
    tokens_used: 51_000,
    tokens_remaining: 149_000,
    pressure_level: "nominal",
  }),
];

// ── Explore — file tree + landing, and the file→symbol→neighborhood chain. ─

export const LIVE_FILES: FileInfo[] = [
  { path: "ministr-core/src/service/query.rs", content_hash: "a1", mtime_ns: 0, section_count: 38 },
  { path: "ministr-core/src/service/mod.rs", content_hash: "a2", mtime_ns: 0, section_count: 21 },
  { path: "ministr-core/src/index/hnsw.rs", content_hash: "a3", mtime_ns: 0, section_count: 44 },
  { path: "ministr-core/src/ingestion/pipeline.rs", content_hash: "a4", mtime_ns: 0, section_count: 52 },
  { path: "ministr-daemon/src/daemon.rs", content_hash: "a5", mtime_ns: 0, section_count: 61 },
  { path: "ministr-api/src/client.rs", content_hash: "a6", mtime_ns: 0, section_count: 49 },
  { path: "ministr-mcp/src/server/tools.rs", content_hash: "a7", mtime_ns: 0, section_count: 33 },
  { path: "ministr-cli/src/commands.rs", content_hash: "a8", mtime_ns: 0, section_count: 40 },
  { path: "ministr-app/src/lib/types.ts", content_hash: "a9", mtime_ns: 0, section_count: 12 },
  { path: "ministr-app/src/components/code/CodeBrowser.tsx", content_hash: "b1", mtime_ns: 0, section_count: 9 },
  { path: "docs/retrieval.md", content_hash: "b2", mtime_ns: 0, section_count: 7 },
  { path: "README.md", content_hash: "b3", mtime_ns: 0, section_count: 5 },
];

const QUERY_RS = `use crate::embed::Embedder;
use crate::index::HnswIndex;
use crate::rerank::Reranker;

/// Semantic search across the corpus. Embeds the query, runs HNSW cosine
/// retrieval, then reranks the top-k before returning cited sections.
pub struct QueryService {
    embedder: Embedder,
    index: HnswIndex,
    reranker: Reranker,
}

impl QueryService {
    /// Run a survey: embed → retrieve → rerank.
    pub async fn survey(
        &self,
        req: &SurveyRequest,
    ) -> Result<SurveyResponse, QueryError> {
        let q = self.embedder.embed(&req.query).await?;
        let hits = self.index.search(&q, req.top_k)?;
        let ranked = self.rerank(hits, req).await?;
        Ok(SurveyResponse { sections: ranked })
    }

    async fn rerank(
        &self,
        hits: Vec<Hit>,
        req: &SurveyRequest,
    ) -> Result<Vec<Section>, QueryError> {
        self.reranker.rerank(hits, &req.query).await
    }
}
`;

const QUERY_RS_PATH = "ministr-core/src/service/query.rs";

const SPANS_FOR_QUERY_RS = [
  { id: `sym-${QUERY_RS_PATH}::QueryService`, name: "QueryService", kind: "struct", signature: "pub struct QueryService", doc_comment: null, line_start: 7, line_end: 11 },
  { id: `sym-${QUERY_RS_PATH}::QueryService::survey`, name: "survey", kind: "function", signature: "pub async fn survey(&self, req: &SurveyRequest) -> Result<SurveyResponse, QueryError>", doc_comment: "Run a survey: embed → retrieve → rerank.", line_start: 15, line_end: 25 },
  { id: `sym-${QUERY_RS_PATH}::QueryService::rerank`, name: "rerank", kind: "function", signature: "async fn rerank(&self, hits: Vec<Hit>, req: &SurveyRequest) -> Result<Vec<Section>, QueryError>", doc_comment: null, line_start: 27, line_end: 33 },
];

/** read_file echoes the requested path so the viewer header matches; the body
 *  is a representative Rust file with clickable symbol spans. */
function readFile(args: Record<string, unknown>): FileContent {
  const path = String(args.path ?? "ministr-core/src/service/query.rs");
  const isTs = path.endsWith(".ts") || path.endsWith(".tsx");
  const isMd = path.endsWith(".md");
  return {
    path,
    lang: isMd ? "markdown" : isTs ? "typescript" : "rust",
    content: QUERY_RS,
    symbol_spans: isMd ? [] : SPANS_FOR_QUERY_RS,
  };
}

const DEFINITION: SymbolDefinitionDetail = {
  id: `sym-${QUERY_RS_PATH}::QueryService::survey`,
  name: "survey",
  kind: "function",
  visibility: "pub",
  signature:
    "pub async fn survey(&self, req: &SurveyRequest) -> Result<SurveyResponse, QueryError>",
  doc_comment:
    "Semantic search across the corpus. Embeds the query, runs HNSW cosine\nretrieval, then reranks the top-k before returning cited sections.",
  file_path: "ministr-core/src/service/query.rs",
  line_start: 15,
  line_end: 25,
  heading_path: ["service", "query", "QueryService", "survey"],
  source_context:
    "pub async fn survey(&self, req: &SurveyRequest) -> Result<SurveyResponse, QueryError> {\n    let q = self.embedder.embed(&req.query).await?;\n    let hits = self.index.search(&q, req.top_k)?;\n    let ranked = self.rerank(hits, req).await?;\n    Ok(SurveyResponse { sections: ranked })\n}",
};

/** symbol_definition echoes the requested id, with the canned body. */
function symbolDefinition(args: Record<string, unknown>): SymbolDefinitionDetail {
  return { ...DEFINITION, id: String(args.symbolId ?? DEFINITION.id) };
}

function ref(from_name: string, from_file: string, ref_kind: string): SymbolRef {
  return {
    from_name,
    from_file,
    to_name: "survey",
    to_file: "ministr-core/src/service/query.rs",
    ref_kind,
  };
}

const REFERENCES: SymbolRef[] = [
  ref("handle_survey", "ministr-daemon/src/daemon.rs", "calls"),
  ref("ask_corpus", "ministr-mcp/src/server/tools.rs", "calls"),
  ref("cmd_survey", "ministr-cli/src/commands.rs", "calls"),
  ref("survey_corpus", "ministr-app/src-tauri/src/commands.rs", "calls"),
  ref("QueryService", "ministr-core/src/service/mod.rs", "implements"),
  ref("use query::QueryService", "ministr-api/src/query.rs", "imports"),
  ref("ministr_survey", "ministr-mcp/src/bridge.rs", "bridge"),
];

function sym(name: string, kind: string, file_path: string): SymbolInfo {
  return {
    id: `sym-${file_path}::${name}`,
    name,
    kind,
    file_path,
    visibility: "pub",
    signature: `${kind} ${name}`,
    doc_comment: null,
    module_path: file_path.replace(/\//g, "::"),
  };
}

const ALL_SYMBOLS: SymbolInfo[] = [
  sym("QueryService", "struct", "ministr-core/src/service/query.rs"),
  sym("survey", "function", "ministr-core/src/service/query.rs"),
  sym("rerank", "function", "ministr-core/src/service/query.rs"),
  sym("SurveyRequest", "struct", "ministr-core/src/service/query.rs"),
  sym("SurveyResponse", "struct", "ministr-core/src/service/query.rs"),
  sym("HnswIndex", "struct", "ministr-core/src/index/hnsw.rs"),
  sym("Embedder", "struct", "ministr-core/src/embed/mod.rs"),
  sym("IngestionPipeline", "struct", "ministr-core/src/ingestion/pipeline.rs"),
  sym("DaemonClient", "struct", "ministr-api/src/client.rs"),
];

/** search_symbols serves both the ⌘K palette (query) and same-file / jump-ref
 *  resolution (filePath). */
function searchSymbols(args: Record<string, unknown>): SymbolInfo[] {
  const query = String(args.query ?? "").toLowerCase();
  const filePath = args.filePath ? String(args.filePath) : null;
  let out = ALL_SYMBOLS;
  if (filePath) out = out.filter((s) => s.file_path === filePath);
  if (query) out = out.filter((s) => s.name.toLowerCase().includes(query));
  return out;
}

const NO_OCCURRENCES: Occurrence[] = [];

// ── Cross-language bridges — the ministr↔ts Tauri seam + the daemon HTTP API. ─

function bridge(over: Partial<BridgeLink> & { kind: string }): BridgeLink {
  return {
    confidence: 0.93,
    export_file: "ministr-app/src-tauri/src/commands.rs",
    export_binding_key: "",
    export_symbol: "",
    export_language: "rust",
    export_line: 1,
    import_file: "ministr-app/src/lib/api.ts",
    import_binding_key: "",
    import_symbol: "",
    import_language: "typescript",
    import_line: 1,
    ...over,
  };
}

const LIVE_BRIDGES: BridgeLink[] = [
  bridge({ kind: "tauri_command", export_symbol: "survey_corpus", import_symbol: "surveyCorpus", export_line: 412, import_line: 88, confidence: 0.96 }),
  bridge({ kind: "tauri_command", export_symbol: "list_sessions", import_symbol: "listSessions", export_line: 980, import_line: 142, confidence: 0.97 }),
  bridge({ kind: "tauri_command", export_symbol: "bridge_query", import_symbol: "bridgeQuery", export_line: 1254, import_line: 203, confidence: 0.9 }),
  bridge({ kind: "tauri_command", export_symbol: "read_file", import_symbol: "readFile", export_line: 640, import_line: 51, confidence: 0.94 }),
  bridge({ kind: "tauri_command", export_symbol: "symbol_definition", import_symbol: "symbolDefinition", export_line: 1102, import_line: 167, confidence: 0.89 }),
  bridge({
    kind: "http_route",
    export_file: "ministr-daemon/src/daemon.rs",
    export_symbol: "GET /api/v1/corpora/{id}/files",
    export_binding_key: "list_files",
    export_line: 1640,
    import_file: "ministr-api/src/client.rs",
    import_symbol: "list_corpus_files",
    import_language: "rust",
    import_line: 349,
    confidence: 0.83,
  }),
  bridge({
    kind: "http_route",
    export_file: "ministr-daemon/src/daemon.rs",
    export_symbol: "GET /api/v1/sessions",
    export_binding_key: "list_sessions",
    export_line: 1480,
    import_file: "ministr-api/src/client.rs",
    import_symbol: "list_sessions",
    import_language: "rust",
    import_line: 526,
    confidence: 0.8,
  }),
  bridge({
    kind: "ffi",
    export_file: "vendor/sqlite/sqlite3.c",
    export_symbol: "sqlite3_open_v2",
    export_language: "c",
    export_line: 178002,
    import_file: "ministr-core/src/storage/sqlite.rs",
    import_symbol: "open",
    import_language: "rust",
    import_line: 64,
    confidence: 0.58,
  }),
];

/** bridge_query honours the kind + file_path filters (the BridgeView "other of
 *  kind" section relies on server-side kind filtering). */
function bridgeQuery(args: Record<string, unknown>): BridgeLink[] {
  const kind = args.kind ? String(args.kind) : null;
  const filePath = args.filePath ? String(args.filePath) : null;
  return LIVE_BRIDGES.filter(
    (b) =>
      (!kind || b.kind === kind) &&
      (!filePath || b.export_file === filePath || b.import_file === filePath),
  );
}

const SUPPORTED_MODELS = [
  { name: "jina-code-v2", dimension: 768, description: "Code-optimised, Matryoshka", code_optimized: true },
  { name: "bge-m3", dimension: 1024, description: "Multilingual dense+sparse", code_optimized: false },
  { name: "nomic-embed-text-v1.5", dimension: 768, description: "General text, Matryoshka", code_optimized: false },
  { name: "all-MiniLM-L6-v2", dimension: 384, description: "Tiny, fast baseline", code_optimized: false },
];

// ── The command → response map handed to withTauriMock. ────────────────────

export const LIVE_FIXTURES: TauriFixtures = {
  // Activity
  list_sessions: LIVE_SESSIONS,
  // Explore
  list_corpus_files: LIVE_FILES,
  read_file: readFile,
  file_occurrences: NO_OCCURRENCES,
  search_symbols: searchSymbols,
  symbol_definition: symbolDefinition,
  symbol_references: REFERENCES,
  bridge_query: bridgeQuery,
  // Tend
  list_supported_models: SUPPORTED_MODELS,
  // Ask
  inference_health: { available: true, reason: "", binary_path: "/usr/local/bin/claude" },
  // Misc idle reads kept empty
  read_corpus_activity: [],
};

// ── Ask history — seeded into localStorage (the thread store), since the
//    conversation rail + pinned answers load from there, not from a command. ─

const ASK_THREADS_KEY = "ministr-ask-threads-v1";
const ASK_PINNED_KEY = "ministr-ask-pinned-v1";

function entry(query: string, answer: string, elapsed_ms: number, mins: number) {
  return {
    query,
    answer,
    source_ids: [
      "ministr-core/src/service/query.rs#survey",
      "docs/retrieval.md#how-survey-works",
    ],
    cached: false,
    model: "claude-opus-4",
    elapsed_ms,
    ts: Date.now() - mins * 60_000,
  };
}

/** Seed a few resumable conversations + pinned answers for the `ministr`
 *  corpus so the Ask history rail and Pinned section render populated. */
export function seedAskThreads(): void {
  try {
    const threads = [
      {
        id: "thread-retrieval",
        corpusId: "ministr",
        createdAt: Date.now() - 40 * 60_000,
        updatedAt: Date.now() - 8 * 60_000,
        turns: [
          {
            id: "t1",
            query: "How does survey rank results?",
            status: "done",
            kind: "qa",
            entry: entry(
              "How does survey rank results?",
              "`survey` embeds the query, runs HNSW cosine retrieval for the top-k, then reranks the candidates with the cross-encoder before returning cited sections [1][2].",
              2140,
              8,
            ),
            unsupported: null,
          },
          {
            id: "t2",
            query: "What happens if the reranker is disabled?",
            status: "done",
            kind: "qa",
            entry: entry(
              "What happens if the reranker is disabled?",
              "Retrieval falls back to raw HNSW cosine order — the top-k hits are returned in similarity order without the cross-encoder pass [1].",
              1760,
              7,
            ),
            unsupported: null,
          },
        ],
      },
      {
        id: "thread-ingestion",
        corpusId: "ministr",
        createdAt: Date.now() - 3 * HOUR,
        updatedAt: Date.now() - 2.4 * HOUR,
        turns: [
          {
            id: "t3",
            query: "Where are sections coalesced during ingestion?",
            status: "done",
            kind: "qa",
            entry: entry(
              "Where are sections coalesced during ingestion?",
              "`coalesce_small_sections` runs in the shared `store_enriched_document` path with a 50-token floor and recurses into children, so per-symbol code sections coalesce too.",
              1980,
              144,
            ),
            unsupported: null,
          },
        ],
      },
    ];
    const pinned = [
      entry(
        "How does survey rank results?",
        "`survey` embeds the query, runs HNSW cosine retrieval, then reranks the top-k before returning cited sections.",
        2140,
        8,
      ),
    ];
    localStorage.setItem(ASK_THREADS_KEY, JSON.stringify({ ministr: threads }));
    localStorage.setItem(ASK_PINNED_KEY, JSON.stringify({ ministr: pinned }));
  } catch {
    /* localStorage unavailable — non-fatal */
  }
}
