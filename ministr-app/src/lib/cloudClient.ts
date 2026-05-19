// Thin typed wrapper over the cloud_* Tauri commands.
//
// SRP: this file converts Tauri invoke results into ergonomic
// promises and types the panel renders against. No React, no DOM —
// keeps it trivially testable.

import { Channel, invoke } from "@tauri-apps/api/core";

export interface CloudStatus {
  configured: boolean;
  authenticated: boolean;
  endpoint: string;
  last_health_ok: boolean | null;
  last_health_latency_ms: number | null;
  last_health_message: string | null;
}

export interface CloudHealth {
  status: string;
  corpus_count: number;
  version: string;
  latency_ms: number;
}

/** Minimal subset of `ministr_api::corpus::CorpusInfo` the panel renders. */
export interface CloudCorpusInfo {
  corpus_id: string;
  paths: string[];
  display_name?: string | null;
  indexing_status?: string | null;
  total_files?: number | null;
  total_chunks?: number | null;
  active_sessions?: number;
}

export interface CloudRegisterResponse {
  corpus_id: string;
  indexing_started: boolean;
}

export interface CloudCloneResponse {
  corpus_id: string;
  cloned: boolean;
  indexing_started: boolean;
  cache_path: string;
}

/**
 * Mirrors `ministr_api::corpus::IngestionProgressEvent` — phase string +
 * counters that get emitted every ~500ms by the SSE stream until the
 * corpus reaches a terminal status.
 */
export interface CloudProgressEvent {
  corpus_id?: string;
  status: number;          // 0 = pending, 1 = running, 2 = complete
  phase: string;           // "idle" | "discovering" | "parsing" | "embedding" | "finalizing"
  files_total?: number;
  files_processed?: number;
  current_file?: string | null;
  estimated_remaining_secs?: number | null;
}

export const cloudClient = {
  status: () => invoke<CloudStatus>("cloud_status"),
  setEndpoint: (endpoint: string) =>
    invoke<void>("cloud_set_endpoint", { endpoint }),
  setBearerToken: (token: string) =>
    invoke<void>("cloud_set_bearer_token", { token }),
  disconnect: () => invoke<void>("cloud_disconnect"),
  healthCheck: () => invoke<CloudHealth>("cloud_health_check"),
  triggerReindex: (corpusId: string) =>
    invoke<string>("cloud_trigger_reindex", { corpusId }),

  // ── Corpus management (mounted on cloud in PR2) ──────────────────────────
  listCorpora: () =>
    invoke<{ corpora: CloudCorpusInfo[] } | CloudCorpusInfo[]>("cloud_list_corpora")
      .then((r): CloudCorpusInfo[] => Array.isArray(r) ? r : r.corpora ?? []),
  registerCorpus: (paths: string[]) =>
    invoke<CloudRegisterResponse>("cloud_register_corpus", { paths }),
  cloneRepo: (repo: string, branch?: string, label?: string) =>
    invoke<CloudCloneResponse>("cloud_clone_repo", { repo, branch, label }),
  unregisterCorpus: (corpusId: string) =>
    invoke<void>("cloud_unregister_corpus", { corpusId }),
  /**
   * Open the SSE progress stream for a corpus on the remote server.
   * Returns the Channel; consumers attach `.onmessage` and let the
   * channel be GC'd when they unmount — the Rust side detects the closed
   * channel and exits the loop.
   */
  corpusProgress: (corpusId: string): Channel<CloudProgressEvent> => {
    const channel = new Channel<CloudProgressEvent>();
    void invoke("cloud_corpus_progress", { corpusId, onEvent: channel }).catch(() => {
      // The Rust side may close the channel on auth/network failure; UI
      // observers see a quiet stop. Logged at debug on the backend.
    });
    return channel;
  },
} as const;
