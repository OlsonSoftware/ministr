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
}

export interface DetectedProject {
  path: string;
  name: string;
}

export type IndexingStatus =
  | { state: "idle" }
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
