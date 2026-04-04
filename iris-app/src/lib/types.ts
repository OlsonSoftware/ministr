export interface CorpusInfo {
  id: string;
  paths: string[];
  status: IndexingStatus;
  files_indexed: number;
  sections_count: number;
  embeddings_count: number;
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
