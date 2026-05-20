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

/**
 * Mirrors `ministr_cloud::UsageResponse` (F1.4 sub-bullet 4). The
 * cloud's `/api/v1/billing/usage` endpoint returns this verbatim and
 * the overview-tile badges (F1.4 sub-bullet 5) render from it.
 */
export interface CloudUsage {
  tenant_id: string;
  /**
   * Resolved billing tier — `"pro" | "team" | "enterprise"`. Mirrors
   * the Rust `ministr_mcp::auth::Plan` enum's `serde(rename_all =
   * "lowercase")` shape. F2.4 — the CloudPanel renders this as the
   * plan badge.
   */
  plan: "pro" | "team" | "enterprise";
  rollups: Array<{ day: string; kind: string; total: number }>;
  today_partial: Array<{ kind: string; total: number }>;
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
  /**
   * Drive the full OAuth 2.1 + PKCE flow against the configured endpoint.
   * Opens the system browser; the user signs in once; the resulting
   * access token is persisted via the same store as `setBearerToken`.
   * Resolves when the token has been saved. Rejects on cancel/timeout
   * (~3 min) or any handshake failure.
   */
  authenticate: () => invoke<void>("cloud_authenticate"),
  /**
   * F1.3 — drive the GitHub-federated sign-in flow against the cloud's
   * `/auth/github/*` routes. The cloud must be configured with
   * `MINISTR_GITHUB_CLIENT_ID` + `MINISTR_GITHUB_CLIENT_SECRET` +
   * `MINISTR_CLOUD_BASE_URL`, otherwise the routes return 404 and the
   * command rejects.
   */
  authenticateGitHub: () => invoke<void>("cloud_authenticate_github"),
  disconnect: () => invoke<void>("cloud_disconnect"),
  healthCheck: () => invoke<CloudHealth>("cloud_health_check"),
  /** F1.4 sub-bullet 5 — fetch the calling tenant's metered usage. */
  billingUsage: () => invoke<CloudUsage>("cloud_billing_usage"),
  /**
   * F2.4 — mint a Stripe Checkout session for the given plan and open
   * it in the system browser. Resolves once the URL has been opened;
   * the actual payment happens in Stripe-hosted UI and the cloud
   * webhook flips `users.plan_id` on success.
   */
  billingCheckout: (plan: "pro" | "team") =>
    invoke<void>("cloud_billing_checkout", { plan }),
  /**
   * F2.4 — mint a Stripe Customer Portal session and open it in the
   * system browser. Invoices, card management, cancellation.
   */
  billingPortal: () => invoke<void>("cloud_billing_portal"),
  triggerReindex: (corpusId: string) =>
    invoke<string>("cloud_trigger_reindex", { corpusId }),

  // ── Corpus management (mounted on cloud in PR2) ──────────────────────────
  listCorpora: () =>
    invoke<{ corpora: CloudCorpusInfo[] } | CloudCorpusInfo[]>("cloud_list_corpora")
      .then((r): CloudCorpusInfo[] => Array.isArray(r) ? r : r.corpora ?? []),
  registerCorpus: (paths: string[]) =>
    invoke<CloudRegisterResponse>("cloud_register_corpus", { paths }),
  /**
   * Clone a remote repo. Pass `githubInstallationId` to use the cloud's
   * GitHub App for private-repo access (F2.1) — the token is minted
   * server-side and never reaches this process.
   */
  cloneRepo: (
    repo: string,
    branch?: string,
    label?: string,
    githubInstallationId?: string,
  ) =>
    invoke<CloudCloneResponse>("cloud_clone_repo", {
      repo,
      branch,
      label,
      githubInstallationId,
    }),
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
