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
  /** Effective embedding model (expert disclosure only — internals
   *  vocabulary never renders above a drill-in). */
  model: string;
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

/** Per-corpus ingestion-progress snapshot (mirrors ministr-api
 *  IngestionProgressInfo): the daemon's live counters, polled point-in-time.
 *  Rate + ETA are NOT on the wire — they derive client-side (lib/progress). */
export interface IngestionProgressInfo {
  corpus_id: string;
  /** 0 = pending, 1 = running, 2 = complete. */
  status: number;
  /** "idle" | "discovering" | "parsing" | "embedding" | "finalizing". */
  phase: string;
  files_total: number;
  files_done: number;
  sections_done: number;
  embeddings_total: number;
  embeddings_done: number;
  /** Relative path of the file being processed; empty string when idle. */
  current_file: string;
}

export function ingestionProgress(): Promise<IngestionProgressInfo[]> {
  return invoke<IngestionProgressInfo[]>("ingestion_progress");
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

export interface FreshnessSummary {
  current: number;
  stale: number;
  new: number;
  missing: number;
  indexing: boolean;
}

export function corpusFreshnessSummary(
  corpusId: string,
): Promise<FreshnessSummary> {
  return invoke<FreshnessSummary>("corpus_freshness_summary", { corpusId });
}

export interface SupportedModel {
  name: string;
  dimension: number;
  description: string;
  code_optimized: boolean;
}

export function listSupportedModels(): Promise<SupportedModel[]> {
  return invoke<SupportedModel[]>("list_supported_models");
}

export function setCorpusConfig(
  corpusId: string,
  model: string | null,
  dimension: number | null,
  rerankDepth: number | null,
): Promise<void> {
  return invoke<void>("set_corpus_config", {
    corpusId,
    model,
    dimension,
    rerankDepth,
  });
}
