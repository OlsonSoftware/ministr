export interface CorpusInfo {
  id: string;
  /** Human-readable label (LCA basename of the registered paths) computed
   *  by the daemon. Older daemons may omit this — UI should fall back to
   *  the basename of the first path. */
  display_name?: string;
  paths: string[];
  status: IndexingStatus;
  files_indexed: number;
  sections_count: number;
  embeddings_count: number;
  active_sessions: number;
  last_indexed?: number;
  symbols_count: number;
  /** Effective embedding model this corpus is indexed + queried with (its
   *  .ministr.toml [corpus] model, else the daemon default). Older daemons
   *  may omit this. */
  model?: string;
  /** gd6: true only for a placeholder the daemon synthesizes for a corpus that
   *  is registered (in the manifest) but not yet *warmed* into memory. After
   *  gd5 the daemon loads corpora in the background, so the UI shows these as
   *  "Warming up…" instead of having them pop into the list once loaded.
   *  Real (loaded) corpora always send `false`/omit it. */
  warming?: boolean;
}

export interface DetectedProject {
  path: string;
  name: string;
}

export type IndexingStatus =
  | { state: "idle" }
  | { state: "queued" }
  | { state: "indexing"; files_done: number; files_total: number }
  | { state: "error"; message: string };

export interface DaemonStatus {
  version: string;
  uptime_secs: number;
  memory_mb: number;
  model: string;
  model_dimension: number;
  corpora: CorpusInfo[];
  log_path?: string;
  total_sessions: number;
  /** Whether the desktop tray is configured to launch at login. Populated
   *  only when running inside the Tauri app; absent from the headless
   *  daemon's HTTP response. */
  autostart_enabled?: boolean;
}

export interface SessionInfo {
  session_id: string;
  current_turn: number;
  tokens_used: number;
  tokens_remaining: number;
  utilization: number;
}

export interface MinistrConfig {
  default_model: string;
  data_dir: string;
}

// ── New types for dashboard features ──

export interface SessionDetail {
  session_id: string;
  corpus_id: string;
  current_turn: number;
  delivered_count: number;
  tokens_used: number;
  tokens_remaining: number;
  utilization: number;
  pressure_level: string;
  // Token economics metrics
  total_deliveries: number;
  cumulative_tokens_delivered: number;
  total_tokens_saved: number;
  total_evictions: number;
  total_compressions: number;
  dedup_hits: number;
  compression_ratio: number;
  /** Token-level split + budget config. Added in a newer daemon — present
   *  only once it has been rebuilt & the MCP session reconnected, so these
   *  are optional and every consumer reads them defensively. */
  cumulative_tokens_evicted?: number;
  cumulative_tokens_compressed?: number;
  delta_updates?: number;
  context_window_tokens?: number;
  pressure_threshold?: number;
  critical_threshold?: number;
  /** Parent session id when this session was created on behalf of a
   *  subagent (e.g. Claude Code's Task tool spawning a sub-claude).
   *  Drives parent/child indenting in tray + SessionDashboard. */
  parent_session_id?: string;
  /** MCP clientInfo.name captured at initialize (e.g. "claude-code"). */
  client_name?: string;
}

export interface FileInfo {
  path: string;
  content_hash: string;
  mtime_ns: number;
  section_count: number;
}

export interface SearchResult {
  content_id: string;
  resolution: string;
  score: number;
  text: string;
  heading_path: string[];
}

export interface SymbolInfo {
  id: string;
  name: string;
  kind: string;
  file_path: string;
  visibility: string;
  signature: string;
  doc_comment: string | null;
  module_path: string;
}

export interface SymbolRef {
  from_name: string;
  from_file: string;
  to_name: string;
  to_file: string;
  ref_kind: string;
}

/** One clickable symbol span within a file, returned by `read_file`.
 *  `signature` + `doc_comment` let the Code surface render a hovercard
 *  with no extra round-trip. Line numbers are 1-based, inclusive. */
export interface SymbolSpan {
  id: string;
  name: string;
  kind: string;
  signature: string;
  doc_comment: string | null;
  line_start: number;
  line_end: number;
}

/** A source file's full contents + the symbol spans the index knows for it,
 *  returned by the `read_file` Tauri command. `lang` is a Shiki language id. */
export interface FileContent {
  path: string;
  lang: string;
  content: string;
  symbol_spans: SymbolSpan[];
}

/** One resolved identifier occurrence in a file (F-CodeExplorer v2 —
 *  click-ANY-token). Present only when the corpus was indexed with occurrence
 *  indexing enabled (`MINISTR_INDEX_OCCURRENCES`); otherwise `file_occurrences`
 *  returns an empty list and the viewer falls back to definition spans. */
export interface Occurrence {
  symbol_id: string;
  name: string;
  byte_start: number;
  byte_end: number;
  line: number;
  col: number;
}

/** A dead-code candidate returned by `dead_code` — a symbol with zero
 *  references that doesn't look like an entry point (the Explore "Unused"
 *  lens). `lines` is the symbol's source span length. */
export interface DeadSymbol {
  symbol_id: string;
  name: string;
  kind: string;
  visibility: string;
  file: string;
  line: number;
  lines: number;
}

// ── Diagnostics (the Explore "Diagnostics" lens — FL5's verify stage) ──

/** Severity of a {@link Diagnostic}, normalised across toolchains (the LSP
 *  DiagnosticSeverity ladder). */
export type DiagnosticSeverity = "error" | "warning" | "info" | "hint";

/** A structured compiler/linter diagnostic returned by the `diagnostics`
 *  command — the agentic VERIFY stage. The project's own toolchain
 *  (cargo / tsc / eslint / ruff / go vet / …, plus any SARIF-emitting tool)
 *  normalised to one shape, never raw build logs. 1-based line/col; `symbol_id`
 *  is the enclosing symbol (the FL1 cross-link) when one exists. */
export interface Diagnostic {
  file: string;
  line_start: number;
  col_start: number;
  line_end: number;
  col_end: number;
  severity: DiagnosticSeverity;
  code: string | null;
  message: string;
  /** The toolchain / tool that produced this diagnostic (e.g. `cargo`, `tsc`). */
  source: string;
  symbol_id: string | null;
}

// ── SOLID / architecture findings (the Explore "Quality" lens) ──

/** Minimal symbol summary embedded inside a `SolidFinding`. */
export interface SolidSymbolRef {
  symbol_id: string;
  name: string;
  kind: string;
  file: string;
  line: number;
}

/** One cohesion component inside an SRP (low-cohesion) finding. */
export interface SolidComponent {
  size: number;
  members: SolidSymbolRef[];
  members_omitted?: number;
}

/** One package→package edge inside a cyclic-dependency finding. */
export interface SolidEdge {
  from: string;
  to: string;
  example_from: SolidSymbolRef;
  example_to: SolidSymbolRef;
}

/** A single SOLID-violation finding (tagged union on `type`), returned by
 *  the `solid_findings` command. `principle` is a snake_case SolidPrinciple. */
export type SolidFinding =
  | {
      type: "redundancy";
      principle: string;
      members: SolidSymbolRef[];
      members_omitted?: number;
      members_total: number;
      canonical: SolidSymbolRef;
      avg_cosine: number;
      avg_jaccard: number;
      cross_module: boolean;
    }
  | {
      type: "low_cohesion";
      principle: string;
      container: SolidSymbolRef;
      components: SolidComponent[];
      method_count: number;
    }
  | {
      type: "fat_interface";
      principle: string;
      interface: SolidSymbolRef;
      method_count: number;
      unused_methods: string[];
      unused_methods_omitted?: number;
      under_using_implementors: SolidSymbolRef[];
      under_using_implementors_omitted?: number;
    }
  | {
      type: "concrete_dependency";
      principle: string;
      consumer: SolidSymbolRef;
      concrete_target: SolidSymbolRef;
      suggested_abstraction: SolidSymbolRef | null;
    }
  | {
      type: "shotgun_surgery";
      principle: string;
      name: string;
      kind: string;
      sites: SolidSymbolRef[];
      sites_omitted?: number;
      sites_total: number;
      avg_jaccard: number;
    }
  | {
      type: "cyclic_dependency";
      principle: string;
      packages: string[];
      edge_count: number;
      example_edges: SolidEdge[];
      example_edges_omitted?: number;
    };

// ── Diff-aware blast radius + blame (the Explore "Changes" lens — FL7) ──

/** One contributor's share of a changed symbol's lines (git blame). */
export interface ChangeAuthor {
  name: string;
  lines: number;
}

/** A symbol a git range touched (the diff seed set), with authorship — from
 *  the `diff_impact` command. `line` is the symbol's start line. */
export interface ChangedSymbol {
  symbol_id: string;
  name: string;
  kind: string;
  file: string;
  line: number;
  /** Contributors to the symbol's lines, most lines first (top 4). */
  authors: ChangeAuthor[];
  /** Author of the most-recently-committed line in the symbol's range. */
  last_author: string | null;
}

/** One node in the union blast radius across all changed symbols. */
export interface ImpactedSymbol {
  symbol_id: string;
  name: string;
  kind: string;
  file: string;
  line: number;
  /** Shallowest call-graph distance from any changed symbol. */
  depth: number;
}

/** Diff-aware blast radius for the Explore "Changes" lens — what a branch
 *  range changed and what it can break. Returned by the `diff_impact` command
 *  (the GUI mirror of FL7's `ministr_impact` range op). */
export interface DiffImpact {
  /** The revision range analysed (e.g. `main..HEAD`). */
  range: string;
  /** Number of changed files that contained indexed symbols. */
  changed_files: number;
  /** Symbols the range touched — the seed set. */
  changed_symbols: ChangedSymbol[];
  /** Distinct symbols in the union blast radius. */
  impacted_symbols: number;
  /** Distinct files in the union blast radius. */
  impacted_files: number;
  /** Distinct test files in the union blast radius. */
  impacted_tests: number;
  /** Aggregate risk of the union (`low` | `medium` | `high`). */
  risk: "low" | "medium" | "high";
  /** The union of impacted nodes (deduped, shallowest depth, bounded). */
  impacted: ImpactedSymbol[];
}

/** Full symbol definition returned by `symbol_definition`. */
export interface SymbolDefinitionDetail {
  id: string;
  name: string;
  kind: string;
  visibility: string;
  signature: string;
  doc_comment: string | null;
  file_path: string;
  line_start: number;
  line_end: number;
  heading_path: string[];
  source_context: string;
}

/** One cross-language bridge link returned by `bridge_query`. */
export interface BridgeLink {
  /** Bridge mechanism (e.g. `tauri_command`, `pyo3_function`, `napi_export`,
   *  `wasm_bindgen`, `http_route`, `ffi`). */
  kind: string;
  confidence: number;
  /** Definition (export) side. */
  export_file: string;
  export_binding_key: string;
  export_symbol: string;
  export_language: string;
  export_line: number;
  /** Call-site (import) side. */
  import_file: string;
  import_binding_key: string;
  import_symbol: string;
  import_language: string;
  import_line: number;
}

export interface IngestionProgressInfo {
  corpus_id: string;
  status: number;
  phase: string;
  files_total: number;
  files_done: number;
  sections_done: number;
  embeddings_total: number;
  embeddings_done: number;
  current_file: string;
}

// ── Activity feed ──

/** One ministr_* tool call as recorded by the daemon. */
export interface ActivityEvent {
  timestamp_ms: number;
  tool: string;
  corpus_id: string;
  session_id?: string;
  summary: string;
  tokens_delta?: number;
  pressure?: string;
  cache_hit: boolean;
  resolution?: string;
  duration_ms: number;
}

/** Result of the `repair_agent_config` command — one idempotent pass
 *  re-scaffolding every AI-assistant config file across all corpus roots. */
export interface RepairReport {
  /** Project roots that were scaffolded/healed. */
  roots: string[];
  /** Newly created files (were missing). */
  created: number;
  /** Stale machine-generated hook files overwritten with the current template. */
  healed: number;
  /** Custom rules injected from `.ministr.toml [agent] rules`. */
  custom_rules: number;
}

/** File-system change the daemon's watcher observed. */
export type CoherenceKind = "created" | "modified" | "removed";

export interface CoherenceEvent {
  timestamp_ms: number;
  corpus_id: string;
  kind: CoherenceKind;
  path: string;
  affected_sections: string[];
  duration_ms: number;
}
