//! Admin endpoint handlers: `/healthz`, `/reindex`, `/reindex/:id/events`.
//!
//! `/healthz` is *unauthenticated* (Azure probe). `/reindex*` are protected
//! by the same Bearer middleware that wraps the MCP routes; the router
//! composition wires that up.

use std::convert::Infallible;
use std::time::Duration;

use axum::Json;
use axum::extract::{Extension, Path, State};
use axum::http::StatusCode;
use axum::response::sse::{Event, KeepAlive, Sse};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tokio_stream::{Stream, StreamExt};
use tracing::{debug, warn};

use super::AdminState;
use super::jobs::{Job, JobTrigger};
use crate::auth::{queue_priority, Tenant};

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

/// F5.5-b-sla-skeleton — `/sla` response shape. Reported as JSON for
/// status-page scrapers (the future `status.ministr.ai`) and richer
/// load-balancer probes.
///
/// `uptime_secs` is `u64` so 6+ years of uptime fits without overflow
/// (the underlying `Instant::elapsed().as_secs()` returns `u64` too).
/// `started_at_iso` lets a scraper compute the boot moment without
/// needing to invert the wall-clock delta itself.
///
/// F5.5-b-latency — `latency` carries p50/p95/p99 over the rolling
/// in-process window. `None` until at least one request has been
/// recorded (the boot's very first `/sla` poll sees `null`).
#[derive(Debug, Serialize)]
pub(super) struct SlaResponse {
    status: &'static str,
    version: &'static str,
    uptime_secs: u64,
    started_at_iso: String,
    latency: Option<LatencyEmission>,
}

/// F5.5-b-latency — JSON-rendered latency envelope. Microseconds get
/// converted to milliseconds at the seam so consumers (status pages,
/// dashboards) read the SLA contract's native unit directly. `count`
/// is the rolling-window sample count for callers that want to
/// understand how warmed-up the percentiles are.
///
/// F5.5-b-persist-read — `window_30d_max_p95_ms` carries the
/// historical worst p95 over the last 30 days from
/// `request_latency_snapshots`. `None` in self-hosted (no DB-backed
/// store wired) or when the rolling window happens to be empty.
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

/// F5.5-b-sla-skeleton — unauthenticated SLA / uptime probe. Foundation
/// for the eventual `status.ministr.ai` dashboard (which polls this
/// endpoint) and for load balancers that want richer state than
/// `/healthz`'s binary up/down.
///
/// Honest scope: this chunk ships uptime only. Latency percentiles
/// (F5.5-b-latency) and cross-pod persistent metrics (F5.5-b-persist)
/// are separate follow-ups.
pub(super) async fn sla_status(State(state): State<AdminState>) -> Json<SlaResponse> {
    use std::time::{SystemTime, UNIX_EPOCH};
    let uptime_secs = state.uptime_secs();
    let now_secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let started_at = now_secs.saturating_sub(uptime_secs);
    // F5.5-b-persist-read — pull the historical 30d max p95 from the
    // wired store (cloud mode only). 30 days = 30 × 86_400 secs.
    // i64 fits centuries of unix-epoch comfortably; saturating_sub
    // defends against the unlikely sub-30d-old epoch boundary.
    let window_30d_max_p95_us = if let Some(store) = state.sla_window_store() {
        let since = i64::try_from(now_secs).unwrap_or(i64::MAX)
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
        started_at_iso: format_unix_secs_iso(started_at),
        latency,
    })
}

/// F5.5-b-sla-skeleton — local copy of the ISO-8601 formatter the
/// F5.4-e-audit CLI uses. `ministr-cloud::audit::civil_from_unix_secs`
/// is module-private; lifting it to a public re-export for one caller
/// isn't worth the surface change. Same Howard Hinnant algorithm; one
/// shared helper across the codebase is a future cleanup chunk.
fn format_unix_secs_iso(secs: u64) -> String {
    // Howard Hinnant's civil_from_days algorithm.
    let days = i64::try_from(secs / 86_400).unwrap_or(0);
    let time = secs % 86_400;
    let hour = time / 3_600;
    let minute = (time % 3_600) / 60;
    let second = time % 60;
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = u64::try_from(z - era * 146_097).unwrap_or(0); // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let y = i64::try_from(yoe).unwrap_or(0) + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = doy - (153 * mp + 2) / 5 + 1; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 }; // [1, 12]
    let year = if m <= 2 { y + 1 } else { y };
    format!("{year:04}-{m:02}-{d:02}T{hour:02}:{minute:02}:{second:02}Z")
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
    let priority = tenant
        .as_ref()
        .map_or(0, |t| queue_priority(t.plan));
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
    Ok((StatusCode::ACCEPTED, Json(ReindexResponse { job_id: job.id })))
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

    #[test]
    fn format_unix_secs_iso_round_trips_known_dates() {
        // 1970-01-01T00:00:00Z — epoch zero.
        assert_eq!(format_unix_secs_iso(0), "1970-01-01T00:00:00Z");
        // 2026-05-22T12:00:00Z — same anchor F5.3-d-iii-b-dispatch
        // documented after the off-by-5-days fix in that chunk's audit
        // tests.
        assert_eq!(format_unix_secs_iso(1_779_451_200), "2026-05-22T12:00:00Z");
    }

    #[test]
    fn format_unix_secs_iso_handles_leap_year() {
        // 2024-02-29T00:00:00Z — leap day. Howard Hinnant's algorithm
        // handles this without special-casing.
        assert_eq!(format_unix_secs_iso(1_709_164_800), "2024-02-29T00:00:00Z");
    }

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
