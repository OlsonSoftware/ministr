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
 * Mirrors `ministr_cloud::orgs::routes::TransferResponse` (F3.2-iv-a).
 * `transferred = true` on a fresh transfer (HTTP 201); `false` on an
 * idempotent re-call against an already-on-target corpus (HTTP 200).
 */
export interface CloudTransferResponse {
  corpus_id: string;
  previous_tenant_id: string;
  new_tenant_id: string;
  transferred: boolean;
}

/**
 * Mirrors `ministr_cloud::orgs::routes::TransferPersonalResponse` (F3.1c-iii).
 * Discriminator-keyed outcome so the UI can render one of three
 * messages without inspecting the optional id.
 */
export interface CloudTransferPersonalResponse {
  /** `"cancelled"` | `"no_active_subscription"` | `"no_personal_customer"`. */
  outcome: string;
  /** Subscription id that was just cancelled. Present only on `outcome = "cancelled"`. */
  subscription_id?: string;
}

/**
 * F6.2-d — mirrors `ministr_mcp::sessions::export::SessionBundleManifest`.
 * Header summary the inspector renders above the per-event tables.
 */
export interface CloudSessionManifest {
  schema_version: number;
  session_id: string;
  opened_at: string;
  exported_at: string;
  budget_used: number;
  delivered_count: number;
  total_delivered_tokens: number;
  /** `"normal"` | `"elevated"` | `"critical"`. */
  pressure_level: string;
}

/**
 * F6.2-d — one row from `delivered.jsonl`. Mirrors the subset of
 * `ministr_core::session::DeliveredItem` the inspector renders.
 */
export interface CloudSessionDelivered {
  content_id: string;
  resolution: string;
  token_count: number;
  turn_delivered: number;
  content_hash: string;
  compression_tier: string;
  compressed_summary?: string;
}

/**
 * F6.2-d — one eviction event from `drops.jsonl`. Mirrors
 * `ministr_api::DropEntry`.
 */
export interface CloudSessionDrop {
  session_id: string;
  tenant_id: string;
  claim_id: string;
  evicted_at: string;
}

/**
 * F6.2-d — parsed bundle the Tauri side returns to the React inspector.
 * `drops` is `undefined` when the cloud's response didn't include a
 * `drops.jsonl` entry (self-hosted serve or no tenant scope); the UI
 * distinguishes "queried but empty" (`[]`) from "not queried"
 * (`undefined`).
 */
export interface CloudSessionBundle {
  manifest: CloudSessionManifest;
  delivered: CloudSessionDelivered[];
  drops?: CloudSessionDrop[];
}

/**
 * F6.2-e — one in-memory session summary returned by
 * `GET /api/v1/sessions`. Mirrors
 * `ministr_mcp::sessions::export::SessionSummary`. v0 lists live
 * in-memory sessions on the contacted pod only; cross-pod listing
 * via the `agent_sessions` Postgres table lands in a future chunk.
 */
export interface CloudSessionSummary {
  session_id: string;
  opened_at: string;
  budget_used: number;
  delivered_count: number;
  total_delivered_tokens: number;
  pressure_level: string;
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

const liveCloudClient = {
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
  /**
   * F3.2-iv-b — transfer the corpus's tenant from the caller's
   * personal account to the target org. Caller must own the corpus
   * AND be owner/admin of the target org. Idempotent against an
   * already-on-target corpus (returns `transferred: false`).
   */
  transferCorpusToOrg: (corpusId: string, orgId: string) =>
    invoke<CloudTransferResponse>("cloud_transfer_corpus_to_org", { corpusId, orgId }),
  transferPersonalSub: (orgId: string) =>
    invoke<CloudTransferPersonalResponse>("cloud_transfer_personal_sub", { orgId }),
  /**
   * F6.2-d — fetch + parse a session bundle for the inspector. POSTs
   * to `/api/v1/sessions/{id}/export`, parses the tar in Rust, returns
   * the structured payload. The Tauri side eats the tar bytes so the
   * JS bundle stays slim (no JS tar parser).
   */
  fetchSessionBundle: (sessionId: string) =>
    invoke<CloudSessionBundle>("cloud_fetch_session_bundle", { sessionId }),
  /**
   * F6.2-e — list in-memory session summaries from the contacted
   * pod's registry. Powers the inspector's session-picker dropdown.
   */
  listSessions: () => invoke<CloudSessionSummary[]>("cloud_list_sessions"),
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

// ── Demo mode ────────────────────────────────────────────────────────────────
//
// CloudPanel is a pure renderer over this client, so a "see real-looking fake
// data" demo lives here, not in the components: when the localStorage flag is
// set, `cloudClient` serves canned, realistic data for every read path and a
// scripted indexing-progress stream, while every mutation is a safe no-op so
// demo can never touch the network. The panel renders unchanged.

const DEMO_KEY = "ministr.cloud.demo";

/** True when CloudPanel should render canned demo data instead of live calls. */
export function isCloudDemo(): boolean {
  try {
    return (
      typeof window !== "undefined" &&
      window.localStorage.getItem(DEMO_KEY) === "1"
    );
  } catch {
    return false;
  }
}

/** Toggle demo mode (persisted to localStorage). */
export function setCloudDemo(on: boolean): void {
  try {
    if (on) window.localStorage.setItem(DEMO_KEY, "1");
    else window.localStorage.removeItem(DEMO_KEY);
  } catch {
    /* private-mode / no storage — demo just won't persist */
  }
}

/** ISO day string `N` days before today (UTC), for plausible rollup series. */
function daysAgo(n: number): string {
  const d = new Date();
  d.setUTCDate(d.getUTCDate() - n);
  return d.toISOString().slice(0, 10);
}

function demoCorpora(): CloudCorpusInfo[] {
  return [
    {
      corpus_id: "acme-platform",
      paths: ["github.com/acme/platform"],
      display_name: "acme/platform",
      indexing_status: "ready",
      total_files: 4821,
      total_chunks: 38104,
      active_sessions: 2,
    },
    {
      corpus_id: "acme-web",
      paths: ["github.com/acme/web"],
      display_name: "acme/web",
      indexing_status: "indexing",
      total_files: 1240,
      total_chunks: 6203,
      active_sessions: 0,
    },
    {
      corpus_id: "design-system",
      paths: ["github.com/acme/design-system"],
      display_name: "acme/design-system",
      indexing_status: "ready",
      total_files: 612,
      total_chunks: 4188,
      active_sessions: 1,
    },
  ];
}

const DEMO_PROGRESS_FILES = ["src/index.ts", "src/router.ts", "src/db/pool.ts", "src/api/auth.ts", "src/ui/App.tsx"];

/** Scripted indexing run the demo `corpusProgress` channel replays. */
function demoProgressScript(): CloudProgressEvent[] {
  const total = 1240;
  const seq: CloudProgressEvent[] = [
    { status: 1, phase: "discovering", files_total: total, files_processed: 0, estimated_remaining_secs: 24 },
  ];
  // Parsing — files climb to the total.
  const parseSteps = [120, 340, 560, 820, 1040, total];
  parseSteps.forEach((done, i) => {
    seq.push({
      status: 1,
      phase: "parsing",
      files_total: total,
      files_processed: done,
      current_file: DEMO_PROGRESS_FILES[i % DEMO_PROGRESS_FILES.length],
      estimated_remaining_secs: Math.max(1, 18 - i * 3),
    });
  });
  // Embedding — files done, GPU phase; eta winds down.
  [6, 3, 1].forEach((eta) => {
    seq.push({
      status: 1,
      phase: "embedding",
      files_total: total,
      files_processed: total,
      estimated_remaining_secs: eta,
    });
  });
  seq.push({ status: 1, phase: "finalizing", files_total: total, files_processed: total, estimated_remaining_secs: 0 });
  seq.push({ status: 2, phase: "idle", files_total: total, files_processed: total, estimated_remaining_secs: 0 });
  return seq;
}

const demoCloudClient: Partial<typeof liveCloudClient> = {
  status: () =>
    Promise.resolve({
      configured: true,
      authenticated: true,
      endpoint: "https://mcp.ministr.ai",
      last_health_ok: true,
      last_health_latency_ms: 38,
      last_health_message: "ok (demo)",
    }),
  healthCheck: () =>
    Promise.resolve({ status: "ok", corpus_count: 3, version: "0.2.1-demo", latency_ms: 38 }),
  billingUsage: () =>
    Promise.resolve({
      tenant_id: "demo-tenant",
      plan: "team" as const,
      rollups: [13, 12, 11, 10, 9, 8, 7, 6, 5, 4, 3, 2, 1].flatMap((n) => [
        { day: daysAgo(n), kind: "survey", total: 40 + ((n * 17) % 60) },
        { day: daysAgo(n), kind: "ask", total: 8 + ((n * 7) % 20) },
      ]),
      today_partial: [
        { kind: "survey", total: 23 },
        { kind: "ask", total: 5 },
      ],
    }),
  listCorpora: () => Promise.resolve(demoCorpora()),
  listOrgs: () =>
    Promise.resolve([
      { id: "org_acme", name: "Acme Engineering", plan_id: "team", role: "owner" },
      { id: "org_oss", name: "Open Source", plan_id: "pro", role: "member" },
    ]),
  listCorpusShares: (corpusId: string) =>
    Promise.resolve([
      {
        corpus_id: corpusId,
        org_id: "org_acme",
        user_id: null,
        scope: "read",
        granted_by: "you@acme.dev",
        created_at: daysAgo(12) + "T10:14:00Z",
      },
    ]),
  listApiKeys: () =>
    Promise.resolve([
      {
        id: "key_live_01",
        name: "CI pipeline",
        prefix: "mst_pk_a1",
        scopes: "ministr:read ministr:write",
        last_used_at: daysAgo(0) + "T08:21:00Z",
        expires_at: null,
        created_at: daysAgo(40) + "T12:00:00Z",
      },
      {
        id: "key_live_02",
        name: "Local laptop",
        prefix: "mst_pk_7f",
        scopes: "ministr:read",
        last_used_at: daysAgo(3) + "T19:02:00Z",
        expires_at: daysAgo(-90) + "T00:00:00Z",
        created_at: daysAgo(70) + "T09:30:00Z",
      },
    ]),
  listWebhookSubs: (orgId: string) =>
    Promise.resolve([
      {
        id: "wh_01",
        org_id: orgId,
        url: "https://acme.dev/hooks/ministr",
        event_filter: "index.completed",
        created_by: "you@acme.dev",
        created_at: daysAgo(20) + "T11:00:00Z",
        last_delivered_at: daysAgo(1) + "T06:45:00Z",
      },
    ]),
  getOrgUsage: (orgId: string, days?: number) =>
    Promise.resolve({
      org_id: orgId,
      range_days: days ?? 30,
      rollups: [5, 4, 3, 2, 1].flatMap((n) => [
        { user_id: "u_amy", email: "amy@acme.dev", day: daysAgo(n), kind: "survey", total: 30 + ((n * 11) % 40) },
        { user_id: "u_ben", email: "ben@acme.dev", day: daysAgo(n), kind: "survey", total: 18 + ((n * 13) % 30) },
      ]),
      today_partial: [
        { user_id: "u_amy", email: "amy@acme.dev", kind: "survey", total: 14 },
        { user_id: "u_ben", email: "ben@acme.dev", kind: "ask", total: 6 },
      ],
    }),
  listSessions: () =>
    Promise.resolve([
      {
        session_id: "sess_8f2a",
        opened_at: daysAgo(0) + "T08:00:00Z",
        budget_used: 0.62,
        delivered_count: 41,
        total_delivered_tokens: 88210,
        pressure_level: "normal",
      },
      {
        session_id: "sess_3c1d",
        opened_at: daysAgo(0) + "T07:12:00Z",
        budget_used: 0.91,
        delivered_count: 73,
        total_delivered_tokens: 142880,
        pressure_level: "elevated",
      },
    ]),
  fetchSessionBundle: (sessionId: string) =>
    Promise.resolve({
      manifest: {
        schema_version: 1,
        session_id: sessionId,
        opened_at: daysAgo(0) + "T08:00:00Z",
        exported_at: daysAgo(0) + "T08:30:00Z",
        budget_used: 0.62,
        delivered_count: 2,
        total_delivered_tokens: 1820,
        pressure_level: "normal",
      },
      delivered: [
        {
          content_id: "acme-platform#src/router.ts",
          resolution: "section",
          token_count: 1024,
          turn_delivered: 1,
          content_hash: "a1b2c3d4",
          compression_tier: "full",
        },
        {
          content_id: "acme-platform#src/db/pool.ts",
          resolution: "claim",
          token_count: 796,
          turn_delivered: 2,
          content_hash: "e5f6a7b8",
          compression_tier: "summary",
          compressed_summary: "Connection-pool sizing + retry/backoff policy.",
        },
      ],
      drops: [],
    }),
  // Mutations — safe no-ops in demo so nothing hits the network.
  setEndpoint: () => Promise.resolve(),
  setBearerToken: () => Promise.resolve(),
  authenticate: () => Promise.resolve(),
  authenticateGitHub: () => Promise.resolve(),
  disconnect: () => {
    setCloudDemo(false);
    return Promise.resolve();
  },
  billingCheckout: () => Promise.resolve(),
  billingPortal: () => Promise.resolve(),
  triggerReindex: () => Promise.resolve("demo-reindex-queued"),
  registerCorpus: (paths: string[]) =>
    Promise.resolve({ corpus_id: paths[0] ?? "demo-corpus", indexing_started: true }),
  cloneRepo: (repo: string) =>
    Promise.resolve({
      corpus_id: repo.replace(/.*\//, "").replace(/\.git$/, "") || "demo-clone",
      cloned: true,
      indexing_started: true,
      cache_path: "/demo/cache",
    }),
  unregisterCorpus: () => Promise.resolve(),
  shareCorpus: (corpusId: string, orgId: string) =>
    Promise.resolve({
      corpus_id: corpusId,
      org_id: orgId,
      user_id: null,
      scope: "read",
      granted_by: "you@acme.dev",
      created_at: new Date().toISOString(),
    }),
  revokeCorpusShare: () => Promise.resolve(),
  transferCorpusToOrg: (corpusId: string, orgId: string) =>
    Promise.resolve({
      corpus_id: corpusId,
      previous_tenant_id: "demo-tenant",
      new_tenant_id: orgId,
      transferred: true,
    }),
  createApiKey: (name: string, scopes?: string) =>
    Promise.resolve({
      id: "key_demo_new",
      name,
      prefix: "mst_pk_de",
      scopes: scopes ?? "ministr:read ministr:write",
      last_used_at: null,
      expires_at: null,
      created_at: new Date().toISOString(),
      token: "mst_pk_demo_0000000000000000000000000000",
    }),
  revokeApiKey: () => Promise.resolve(),
  createWebhookSub: (orgId: string, webhookUrl: string, eventFilter?: string) =>
    Promise.resolve({
      id: "wh_demo_new",
      org_id: orgId,
      url: webhookUrl,
      event_filter: eventFilter ?? "*",
      created_by: "you@acme.dev",
      created_at: new Date().toISOString(),
      last_delivered_at: null,
      secret: "whsec_demo_0000000000000000",
    }),
  deleteWebhookSub: () => Promise.resolve(),
  testWebhookSub: () => Promise.resolve({ final_status: 200, attempts: 1, succeeded: true }),
  exportOrgUsageCsv: () => Promise.resolve(null),
  corpusProgress: (corpusId: string): Channel<CloudProgressEvent> => {
    // A plain object whose `onmessage` we can read back to drive emits —
    // avoids depending on Tauri Channel internals. ProgressDrawer only sets
    // `.onmessage`, so this is structurally sufficient.
    const fake = { onmessage: null } as unknown as Channel<CloudProgressEvent>;
    const script = demoProgressScript().map((e) => ({ ...e, corpus_id: corpusId }));
    let i = 0;
    const timer = setInterval(() => {
      const handler = (fake as { onmessage?: (e: CloudProgressEvent) => void }).onmessage;
      if (!handler) return; // consumer hasn't attached yet — wait
      handler(script[i]);
      i += 1;
      if (i >= script.length) clearInterval(timer);
    }, 650);
    return fake;
  },
};

/**
 * The exported client. In demo mode, reads + the progress stream come from
 * `demoCloudClient`; everything else falls through to the live Tauri-backed
 * implementation. Typed as the live shape so call sites are unchanged.
 */
export const cloudClient: typeof liveCloudClient = new Proxy(liveCloudClient, {
  get(target, prop, receiver) {
    if (isCloudDemo()) {
      const demo = demoCloudClient as Record<string, unknown>;
      if (typeof prop === "string" && prop in demo) {
        return demo[prop];
      }
    }
    return Reflect.get(target, prop, receiver);
  },
});
