export interface CorpusInfo {
  id: string;
  paths: string[];
  status: IndexingStatus;
  files_indexed: number;
  sections_count: number;
  embeddings_count: number;
  active_sessions: number;
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

export interface IrisConfig {
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
  files_total: number;
  files_done: number;
  embeddings_done: number;
}
