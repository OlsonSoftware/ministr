//! Admin endpoint handlers: `/healthz`, `/reindex`, `/reindex/:id/events`.
//!
//! `/healthz` is *unauthenticated* (Azure probe). `/reindex*` are protected
//! by the same Bearer middleware that wraps the MCP routes; the router
//! composition wires that up.

use std::convert::Infallible;
use std::time::Duration;

use axum::Json;
use axum::extract::{Extension, Path, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tokio_stream::{Stream, StreamExt};
use tracing::{debug, warn};

use super::AdminState;
use super::jobs::{Job, JobTrigger};
use crate::auth::{Tenant, queue_priority};

#[derive(Debug, Serialize)]
pub(super) struct HealthResponse {
    status: &'static str,
    corpus_count: usize,
    version: &'static str,
}

/// Unauthenticated health probe. Always returns 200 unless the process is
/// genuinely unwell; ACA / load balancers can use this to gate traffic.
pub(super) async fn healthz(State(state): State<AdminState>) -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ready",
        corpus_count: state.corpus_count(),
        version: env!("CARGO_PKG_VERSION"),
    })
}

/// `/sla` response shape. Reported as JSON for status-page scrapers
/// (the future `status.ministr.ai`) and richer load-balancer probes.
///
/// `uptime_secs` is `u64` so 6+ years of uptime fits without overflow
/// (the underlying `Instant::elapsed().as_secs()` returns `u64` too).
/// `started_at_iso` lets a scraper compute the boot moment without
/// needing to invert the wall-clock delta itself.
///
/// `latency` carries p50/p95/p99 over the rolling in-process window.
/// `None` until at least one request has been recorded (the boot's
/// very first `/sla` poll sees `null`).
#[derive(Debug, Serialize)]
pub(super) struct SlaResponse {
    status: &'static str,
    version: &'static str,
    uptime_secs: u64,
    started_at_iso: String,
    latency: Option<LatencyEmission>,
}

/// JSON-rendered latency envelope. Microseconds get converted to
/// milliseconds at the seam so consumers (status pages, dashboards)
/// read the SLA contract's native unit directly. `count` is the
/// rolling-window sample count for callers that want to understand how
/// warmed-up the percentiles are.
///
/// `window_30d_max_p95_ms` carries the historical worst p95 over the
/// last 30 days from `request_latency_snapshots`. `None` in
/// self-hosted (no DB-backed store wired) or when the rolling window
/// happens to be empty.
#[derive(Debug, Serialize)]
struct LatencyEmission {
    count: usize,
    p50_ms: u64,
    p95_ms: u64,
    p99_ms: u64,
    window_30d_max_p95_ms: Option<u64>,
}

/// Construct a [`LatencyEmission`] from the in-memory snapshot and the
/// optional historical max. Kept out of `From` because of the two-arg
/// shape.
fn latency_emission(
    s: crate::admin::LatencySnapshot,
    window_30d_max_p95_us: Option<u32>,
) -> LatencyEmission {
    LatencyEmission {
        count: s.count,
        p50_ms: u64::from(s.p50_us) / 1_000,
        p95_ms: u64::from(s.p95_us) / 1_000,
        p99_ms: u64::from(s.p99_us) / 1_000,
        window_30d_max_p95_ms: window_30d_max_p95_us.map(|us| u64::from(us) / 1_000),
    }
}

/// Unauthenticated SLA / uptime probe. Foundation for the eventual
/// `status.ministr.ai` dashboard (which polls this endpoint) and for
/// load balancers that want richer state than `/healthz`'s binary
/// up/down.
pub(super) async fn sla_status(State(state): State<AdminState>) -> Json<SlaResponse> {
    use std::time::{SystemTime, UNIX_EPOCH};
    let uptime_secs = state.uptime_secs();
    let now_secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let started_at = now_secs.saturating_sub(uptime_secs);
    let started_at_iso = ministr_api::format_unix_secs_iso(started_at);
    // Pull the historical 30d max p95 from the wired store (cloud mode
    // only). 30 days = 30 × 86_400 secs. i64 fits centuries of
    // unix-epoch comfortably; saturating_sub defends against the
    // unlikely sub-30d-old epoch boundary.
    let window_30d_max_p95_us = if let Some(store) = state.sla_window_store() {
        let since = i64::try_from(now_secs)
            .unwrap_or(i64::MAX)
            .saturating_sub(30 * 86_400);
        match store.max_p95_since(since).await {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!(error = %e, "sla window store query failed; rendering null");
                None
            }
        }
    } else {
        None
    };
    let latency = state
        .latency_tracker()
        .snapshot()
        .map(|s| latency_emission(s, window_30d_max_p95_us));
    Json(SlaResponse {
        status: "ready",
        version: env!("CARGO_PKG_VERSION"),
        uptime_secs,
        started_at_iso,
        latency,
    })
}

/// Public HTTP endpoint serving the operator's revocation JSONL.
///
/// On-prem customers can optionally fetch the operator's
/// `revoke-license`-managed JSONL from this endpoint instead of
/// mounting the file directly. Three states:
///
/// - `MINISTR_LICENSE_REVOCATIONS_SERVE_PATH` unset → 404 +
///   plain-text body explaining the operator hasn't opted in.
/// - env set + file readable → 200 + `application/x-ndjson` body +
///   `Cache-Control: max-age=300, public` so polling clients don't
///   thunder.
/// - env set + file unreadable → 503 + plain-text error so the
///   operator sees the misconfiguration.
///
/// **Reads the env var at request time** rather than boot so an
/// operator can swap the file path or update its contents without
/// bouncing the serve — critical for revocation latency.
///
/// **Unauthenticated**: the revocation list is non-secret. A
/// `jwt_id_hash` reveals "this license is revoked" but nothing
/// about the bearer, the customer, or the original mint context.
/// Customers need to fetch it without bearer tokens for the deferred
/// revocation-api-fetch flow to work.
pub(super) async fn serve_revocation_list() -> Response {
    let Ok(path) = std::env::var("MINISTR_LICENSE_REVOCATIONS_SERVE_PATH") else {
        return (
            StatusCode::NOT_FOUND,
            "MINISTR_LICENSE_REVOCATIONS_SERVE_PATH is not set on this serve. \
             The operator has not opted in to HTTP-served revocation lists; \
             use the file-based MINISTR_LICENSE_REVOCATIONS env var instead.",
        )
            .into_response();
    };
    if path.trim().is_empty() {
        return (
            StatusCode::NOT_FOUND,
            "MINISTR_LICENSE_REVOCATIONS_SERVE_PATH is set but empty.",
        )
            .into_response();
    }
    let body = match std::fs::read(&path) {
        Ok(b) => b,
        Err(e) => {
            warn!(
                error = %e,
                path = %path,
                "MINISTR_LICENSE_REVOCATIONS_SERVE_PATH points at an unreadable file"
            );
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                format!(
                    "revocation list at {path} unreadable: {e}; \
                     check operator config + filesystem permissions",
                ),
            )
                .into_response();
        }
    };
    let mut response = (StatusCode::OK, body).into_response();
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/x-ndjson"),
    );
    // 5-minute cache is a thundering-herd guard; the customer-side
    // fetcher handles freshness by polling more often if it has reason
    // to. Public because the list is non-secret and CDN-friendly.
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("public, max-age=300"),
    );
    response
}

#[derive(Debug, Deserialize)]
pub(super) struct ReindexRequest {
    corpus_id: String,
}

#[derive(Debug, Serialize)]
pub(super) struct ReindexResponse {
    job_id: String,
}

/// Enqueue a new reindex job.
///
/// `tenant` is `Option` so the same handler powers BOTH the cloud
/// (where token-validation middleware always populates it) and
/// self-hosted serve without auth (where the extension is absent and
/// every job lands in the default priority bucket).
pub(super) async fn reindex(
    State(state): State<AdminState>,
    tenant: Option<Extension<Tenant>>,
    Json(req): Json<ReindexRequest>,
) -> Result<(StatusCode, Json<ReindexResponse>), (StatusCode, String)> {
    let priority = tenant.as_ref().map_or(0, |t| queue_priority(t.plan));
    let job = state
        .queue
        .enqueue(req.corpus_id, JobTrigger::Manual, priority)
        .await
        .map_err(|e| {
            warn!(error = %e, "failed to enqueue reindex job");
            (StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
        })?;
    debug!(
        job_id = %job.id,
        corpus_id = %job.corpus_id,
        priority,
        "enqueued reindex job"
    );
    Ok((
        StatusCode::ACCEPTED,
        Json(ReindexResponse { job_id: job.id }),
    ))
}

/// SSE stream of job-progress snapshots.
///
/// Emits one event every 500ms (or sooner when state changes via DB poll).
/// Closes the stream when the job reaches a terminal status.
pub(super) async fn reindex_events(
    State(state): State<AdminState>,
    Path(job_id): Path<String>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, (StatusCode, String)> {
    // Bail early on unknown id so clients see 404 rather than an empty stream.
    match state.queue.get(&job_id).await {
        Ok(Some(_)) => {}
        Ok(None) => return Err((StatusCode::NOT_FOUND, format!("unknown job: {job_id}"))),
        Err(e) => return Err((StatusCode::INTERNAL_SERVER_ERROR, e.to_string())),
    }

    let (tx, rx) = mpsc::channel::<Job>(16);
    let queue = state.queue.clone();
    let id = job_id.clone();
    tokio::spawn(async move {
        let mut last_updated = 0_u64;
        loop {
            match queue.get(&id).await {
                Ok(Some(job)) => {
                    if job.updated_at != last_updated {
                        last_updated = job.updated_at;
                        if tx.send(job.clone()).await.is_err() {
                            break;
                        }
                    }
                    if job.status.is_terminal() {
                        break;
                    }
                }
                Ok(None) => {
                    debug!(job_id = %id, "job disappeared mid-stream");
                    break;
                }
                Err(e) => {
                    warn!(error = %e, "sse poll failed");
                    break;
                }
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    });

    let stream = ReceiverStream::new(rx).map(|job| {
        let payload = serde_json::to_string(&job).unwrap_or_else(|_| "{}".to_string());
        Ok(Event::default().event("progress").data(payload))
    });

    Ok(Sse::new(stream).keep_alive(KeepAlive::default()))
}

#[cfg(test)]
mod tests {
    use super::*;

    // The two algorithmic round-trip tests
    // (`format_unix_secs_iso_round_trips_known_dates` +
    // `format_unix_secs_iso_handles_leap_year`) moved to
    // `ministr-api/src/iso8601.rs` when the helper consolidated
    // into the workspace-shared location. The handler-shape test
    // below still pins the JSON envelope.

    #[tokio::test]
    async fn sla_handler_returns_uptime_at_least_zero() {
        use crate::admin::AdminState;
        let state = AdminState::in_memory(None);
        let resp = sla_status(axum::extract::State(state)).await;
        // Body type is Json<SlaResponse>; deref to inspect.
        let body = resp.0;
        assert_eq!(body.status, "ready");
        assert_eq!(body.version, env!("CARGO_PKG_VERSION"));
        // uptime_secs is u64; just confirm the field exists + the iso
        // round-trip didn't panic. After a `tokio::time::sleep` we'd
        // expect a non-zero value but the test would be timing-sensitive.
        assert!(body.started_at_iso.ends_with('Z'));
        assert_eq!(body.started_at_iso.len(), 20); // "1970-01-01T00:00:00Z" = 20 chars
    }
}
