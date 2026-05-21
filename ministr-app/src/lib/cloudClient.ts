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

/**
 * Mirrors `ministr_cloud::orgs::routes::OrgSummary` (F3.1a). One org
 * the calling user is a member of, with their role inside it.
 */
export interface CloudOrg {
  id: string;
  name: string;
  plan_id: string;
  /** `"owner" | "admin" | "member"` — the caller's role in the org. */
  role: string;
}

/**
 * Mirrors `ministr_cloud::orgs::corpus_acl::AclEntry` (F3.2-i). One grant on
 * a corpus. v0 only mints org-side grants — `user_id` is reserved for future.
 */
export interface CloudAclEntry {
  corpus_id: string;
  org_id: string | null;
  user_id: string | null;
  scope: string;
  granted_by: string;
  /** ISO-8601 UTC. */
  created_at: string;
}

/**
 * Mirrors `ministr_cloud::api_keys::ApiKeyRow` (F3.4a). One row in the
 * caller's active-keys list — secret fields (`hash`, raw token) are
 * deliberately absent; only `prefix` (first 8 chars of the random
 * portion) is shown so the user can label the row in their UI.
 */
export interface CloudApiKey {
  id: string;
  name: string;
  prefix: string;
  scopes: string;
  last_used_at: string | null;
  expires_at: string | null;
  created_at: string;
}

/**
 * `POST /api/v1/api_keys` response (F3.4a). The `token` field carries
 * the raw `mst_pk_…` bearer EXACTLY ONCE — the cloud cannot recover
 * it after this response. Callers must display + copy it immediately
 * and never store or log it.
 */
export interface CloudCreatedApiKey extends CloudApiKey {
  token: string;
}

/**
 * Mirrors `ministr_cloud::webhooks::WebhookSubscription` (F3.5a).
 * The signing `secret` is intentionally absent — only the one-time
 * create response (see [`CloudCreatedWebhookSub`]) carries it.
 */
export interface CloudWebhookSub {
  id: string;
  org_id: string;
  url: string;
  event_filter: string;
  created_by: string;
  created_at: string;
  last_delivered_at: string | null;
}

/**
 * `POST /api/v1/orgs/{id}/webhooks` response (F3.5a). The `secret`
 * field carries the raw HMAC signing secret EXACTLY ONCE — the cloud
 * cannot recover it after this response.
 */
export interface CloudCreatedWebhookSub extends CloudWebhookSub {
  secret: string;
}

/**
 * `POST .../webhooks/{wid}/test` response. Outcome of one synthetic
 * delivery against the subscription's URL.
 */
export interface CloudWebhookTestResult {
  final_status: number | null;
  attempts: number;
  succeeded: boolean;
}

/**
 * Mirrors `ministr_cloud::orgs::OrgRollupRow` (F3.3a). One per-day,
 * per-kind, per-member rollup row.
 */
export interface CloudOrgRollupRow {
  user_id: string;
  email: string;
  day: string;
  kind: string;
  total: number;
}

/**
 * Mirrors `ministr_cloud::orgs::OrgPartialRow` (F3.3a). Today's
 * not-yet-rolled-up events summed per (member, kind).
 */
export interface CloudOrgPartialRow {
  user_id: string;
  email: string;
  kind: string;
  total: number;
}

/**
 * Mirrors `ministr_cloud::orgs::OrgUsageResponse` (F3.3a). Per-member
 * usage breakdown for the F3.3b dashboard.
 */
export interface CloudOrgUsage {
  org_id: string;
  range_days: number;
  rollups: CloudOrgRollupRow[];
  today_partial: CloudOrgPartialRow[];
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
  /** F3.1a — list orgs the caller is a member of, with their role. */
  listOrgs: () => invoke<CloudOrg[]>("cloud_list_orgs"),
  /**
   * F3.2-ii — grant an org read access to a corpus. Caller must own the
   * corpus AND be a member of the target org; the cloud enforces both.
   */
  shareCorpus: (corpusId: string, orgId: string) =>
    invoke<CloudAclEntry>("cloud_share_corpus", { corpusId, orgId }),
  /** F3.2-ii — list current ACL grants on a corpus (owner-only). */
  listCorpusShares: (corpusId: string) =>
    invoke<CloudAclEntry[]>("cloud_list_corpus_shares", { corpusId }),
  /** F3.2-ii — revoke an org's grant. Idempotent on the server side. */
  revokeCorpusShare: (corpusId: string, orgId: string) =>
    invoke<void>("cloud_revoke_corpus_share", { corpusId, orgId }),
  /** F3.4b — list the caller's active service-account API keys. */
  listApiKeys: () => invoke<CloudApiKey[]>("cloud_list_api_keys"),
  /**
   * F3.4b — mint a new service-account API key. The returned `token`
   * is the raw `mst_pk_…` bearer; the cloud never returns it again.
   * Default scopes are `"ministr:read ministr:write"` when omitted.
   */
  createApiKey: (name: string, scopes?: string) =>
    invoke<CloudCreatedApiKey>("cloud_create_api_key", { name, scopes }),
  /** F3.4b — soft-revoke a key. */
  revokeApiKey: (keyId: string) =>
    invoke<void>("cloud_revoke_api_key", { keyId }),
  /** F3.5b-ii — list webhook subscriptions for an org. */
  listWebhookSubs: (orgId: string) =>
    invoke<CloudWebhookSub[]>("cloud_list_webhook_subs", { orgId }),
  /**
   * F3.5b-ii — mint a webhook subscription. Returns the one-time HMAC
   * signing secret; callers MUST surface it immediately and never
   * persist it.
   */
  createWebhookSub: (orgId: string, webhookUrl: string, eventFilter?: string) =>
    invoke<CloudCreatedWebhookSub>("cloud_create_webhook_sub", {
      orgId,
      webhookUrl,
      eventFilter,
    }),
  /** F3.5b-ii — remove a subscription. */
  deleteWebhookSub: (orgId: string, subscriptionId: string) =>
    invoke<void>("cloud_delete_webhook_sub", { orgId, subscriptionId }),
  /** F3.5b-ii — fire a synthetic `ministr.test` payload at the receiver. */
  testWebhookSub: (orgId: string, subscriptionId: string) =>
    invoke<CloudWebhookTestResult>("cloud_test_webhook_sub", {
      orgId,
      subscriptionId,
    }),
  /**
   * F3.3b — fetch per-member usage rollups for an org. Owner/admin
   * only on the server side; non-privileged callers see 403. Default
   * window is 30 days; pass `days` to override (clamped server-side
   * to [1, 366]).
   */
  getOrgUsage: (orgId: string, days?: number) =>
    invoke<CloudOrgUsage>("cloud_get_org_usage", { orgId, days }),
  /**
   * F3.3c — fetch the same org usage data as `getOrgUsage` but rendered
   * as RFC-4180 CSV, then prompt the user with a native Save dialog and
   * write the file. Resolves with the saved path, or `null` if the
   * dialog was cancelled. Owner/admin only on the server side.
   */
  exportOrgUsageCsv: (orgId: string, days?: number) =>
    invoke<string | null>("cloud_export_org_usage_csv", { orgId, days }),
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
