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
