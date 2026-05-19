// Thin typed wrapper over the cloud_* Tauri commands.
//
// SRP: this file converts Tauri invoke results into ergonomic
// promises and types the panel renders against. No React, no DOM —
// keeps it trivially testable.

import { invoke } from "@tauri-apps/api/core";

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
} as const;
