// F5.5-b-page-skeleton — typed mirror of the `/sla` JSON shape +
// server-side fetcher with revalidate.
//
// Source of truth: `ministr-mcp/src/admin/handlers.rs` `SlaResponse`
// (plus `LatencyEmission`). Keep in sync when fields change.

export interface SlaLatency {
  count: number;
  p50_ms: number;
  p95_ms: number;
  p99_ms: number;
  /**
   * F5.5-b-persist-read — historical "worst p95 in the last 30 days"
   * surfaced from `request_latency_snapshots`. Null on self-hosted
   * serve (no DB-backed store wired) or when the window is empty.
   */
  window_30d_max_p95_ms: number | null;
}

export interface SlaResponse {
  status: 'ready';
  /** Server crate version, e.g. "0.6.0". */
  version: string;
  /** Seconds since the AdminState was constructed. */
  uptime_secs: number;
  /** ISO-8601 UTC timestamp of boot. */
  started_at_iso: string;
  /**
   * F5.5-b-latency — current rolling-window percentiles. Null on
   * fresh boots before any request has flowed through the
   * `record_latency_middleware`.
   */
  latency: SlaLatency | null;
}

/**
 * Server-side fetch of `/sla` from the configured cloud endpoint.
 *
 * Returns `null` on any error (unreachable backend, malformed JSON,
 * non-2xx HTTP) so the calling page can render a graceful degraded
 * state rather than 500ing.
 *
 * Cached for 30 seconds via Next.js fetch revalidation — fresh enough
 * for a status page, light enough on the backend that polling tabs
 * don't synchronise into a thundering herd.
 */
export async function fetchSlaStatus(
  baseUrl: string,
): Promise<SlaResponse | null> {
  try {
    const url = `${baseUrl.replace(/\/$/, '')}/sla`;
    const res = await fetch(url, { next: { revalidate: 30 } });
    if (!res.ok) {
      return null;
    }
    const data = (await res.json()) as SlaResponse;
    // Defensive: every required field must be present + the right
    // shape; otherwise fall through to null. Protects against
    // protocol drift between docs-next and the cloud serve.
    if (
      data.status !== 'ready' ||
      typeof data.version !== 'string' ||
      typeof data.uptime_secs !== 'number' ||
      typeof data.started_at_iso !== 'string'
    ) {
      return null;
    }
    return data;
  } catch {
    return null;
  }
}

/**
 * Default cloud base URL. Override with
 * `NEXT_PUBLIC_MINISTR_CLOUD_BASE_URL` at build time for staging
 * or local-dev deployments of this page.
 */
export const DEFAULT_CLOUD_BASE_URL = 'https://mcp.ministr.ai';

/** Human-readable uptime formatter. */
export function formatUptime(secs: number): string {
  if (secs < 60) return `${secs}s`;
  if (secs < 3600) {
    const m = Math.floor(secs / 60);
    return `${m}m ${secs % 60}s`;
  }
  if (secs < 86_400) {
    const h = Math.floor(secs / 3600);
    const m = Math.floor((secs % 3600) / 60);
    return `${h}h ${m}m`;
  }
  const d = Math.floor(secs / 86_400);
  const h = Math.floor((secs % 86_400) / 3600);
  return `${d}d ${h}h`;
}
