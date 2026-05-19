//! Compose admin endpoints into axum `Router`s — split into **public** and
//! **protected** so the caller can wrap them with different auth layers.
//!
//! - **Public** (`admin_public_routes`): `/healthz` (no auth — ACA probe)
//!   and `/webhook/github` (HMAC-authenticated inside the handler itself,
//!   *not* OAuth).
//! - **Protected** (`admin_protected_routes`): `/reindex` and the SSE
//!   progress stream. The caller wraps this with bearer-token middleware
//!   so reindex triggers require an authenticated MCP client.

use axum::Router;
use axum::routing::{get, post};

use super::AdminState;
use super::handlers::{healthz, reindex, reindex_events};
use super::webhook::github_webhook;

/// Build the public admin router: health probe + GitHub webhook.
pub fn admin_public_routes(state: AdminState) -> Router {
    Router::new()
        .route("/healthz", get(healthz))
        .route("/webhook/github", post(github_webhook))
        .with_state(state)
}

/// Build the OAuth-protected admin router: reindex + SSE progress.
pub fn admin_protected_routes(state: AdminState) -> Router {
    Router::new()
        .route("/reindex", post(reindex))
        .route("/reindex/{job_id}/events", get(reindex_events))
        .with_state(state)
}
