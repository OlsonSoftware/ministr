import { invoke } from "@tauri-apps/api/core";

/**
 * Typed IPC surface — every daemon fact the screens consume comes
 * through here. Shapes mirror ministr-api (the Rust side of the seam).
 */

export interface CorpusInfo {
  id: string;
  display_name: string;
  paths: string[];
  status: unknown;
  files_indexed: number;
  sections_count: number;
  active_sessions: number;
}

export type FreshnessState = "current" | "stale" | "new" | "missing";

export interface FileFreshness {
  path: string;
  state: FreshnessState;
}

export interface FreshnessResponse {
  files: FileFreshness[];
  indexing: boolean;
}

export function listCorpora(): Promise<CorpusInfo[]> {
  return invoke<CorpusInfo[]>("list_corpora");
}

export function corpusFreshness(corpusId: string): Promise<FreshnessResponse> {
  return invoke<FreshnessResponse>("corpus_freshness", { corpusId });
}

export function triggerReindex(corpusId: string): Promise<void> {
  return invoke<void>("trigger_reindex", { corpusId });
}

export interface ActivityEvent {
  timestamp_ms: number;
  tool: string;
  corpus_id: string;
  session_id?: string;
  summary: string;
  tokens_delta?: number;
  cache_hit: boolean;
}

export interface OutcomeEventInfo {
  session_id: string;
  path: string;
  read_rank: number;
  first_touch: boolean;
  reads_before: number;
  edited_at_ms: number;
}

export interface SessionOutcomeInfo {
  session_id: string;
  distinct_reads: number;
  joins: number;
  first_touch_hits: number;
}

export interface OutcomesResponse {
  events: OutcomeEventInfo[];
  stats: SessionOutcomeInfo[];
}

export function recentActivity(limit?: number): Promise<ActivityEvent[]> {
  return invoke<ActivityEvent[]>("recent_activity", { limit });
}

export function corpusOutcomes(corpusId: string): Promise<OutcomesResponse> {
  return invoke<OutcomesResponse>("corpus_outcomes", { corpusId });
}

export interface RegisterCorpusResponse {
  corpus_id: string;
  indexing_started: boolean;
}

export function pickProjectFolder(): Promise<string | null> {
  return invoke<string | null>("pick_project_folder");
}

export function registerCorpus(paths: string[]): Promise<RegisterCorpusResponse> {
  return invoke<RegisterCorpusResponse>("register_corpus", { paths });
}

export interface IndexedSectionInfo {
  heading: string;
  text: string;
}

export interface IndexedFileResponse {
  sections: IndexedSectionInfo[];
  found: boolean;
}

export interface FileContent {
  content: string;
  symbols: unknown[];
}

export function indexedFile(
  corpusId: string,
  path: string,
): Promise<IndexedFileResponse> {
  return invoke<IndexedFileResponse>("indexed_file", { corpusId, path });
}

export function readFile(corpusId: string, path: string): Promise<FileContent> {
  return invoke<FileContent>("read_file", { corpusId, path });
}
