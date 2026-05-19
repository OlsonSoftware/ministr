//! Compose the admin endpoints into a single axum `Router`.
//!
//! Health is public (used by ACA probes). The reindex routes and the
//! GitHub webhook are mounted as-is; the *caller* is expected to layer
//! authentication on top — typically by merging this router into the
//! protected MCP router via `auth::protected_router`.

use axum::Router;
use axum::routing::{get, post};

use super::AdminState;
use super::handlers::{healthz, reindex, reindex_events};
use super::webhook::github_webhook;

/// Build the admin router exposing `/healthz`, `/reindex`, the SSE event
/// stream, and the GitHub webhook handler.
pub fn admin_routes(state: AdminState) -> Router {
    Router::new()
        .route("/healthz", get(healthz))
        .route("/reindex", post(reindex))
        .route("/reindex/{job_id}/events", get(reindex_events))
        .route("/webhook/github", post(github_webhook))
        .with_state(state)
}
