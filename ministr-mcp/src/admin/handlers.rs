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
