//! `GET /atlas/*` axum routes.
//!
//! Two surfaces:
//!
//! - `GET /atlas/manifest.json` — the public manifest mirror of
//!   [`ATLAS_SEED_REPOS`]. F2.6 v0 returns the seed-list view; F4.1
//!   replaces it with the live-blob-set view (same shape, runtime
//!   fields populated).
//! - `GET /atlas/{slug}/{survey|symbols|references|read|extract|toc}`
//!   — the per-repo query routes. F2.6 v0 returns
//!   `503 Service Unavailable` with a JSON `not_indexed_yet` payload
//!   because the weekly cron that produces blobs ships in F4.2; the
//!   routes exist so the cloud surface area is stable for clients
//!   ahead of the worker landing.
//!
//! Mounting is the caller's responsibility — [`atlas_routes`] returns
//! an unwrapped `Router` so `cmd_serve_http` can layer the
//! per-Atlas quota check (uses the existing `AtlasAccessRule` +
//! F1.4 `atlas.queries` event kind) outside this module.

use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    routing::get,
};
use serde::Serialize;

use crate::manifest::ManifestSnapshot;
use crate::repos::ATLAS_SEED_REPOS;

/// State the Atlas routes close over. Today's only field is the
/// manifest snapshot; F4.2 adds the live blob index. Cheap to clone
/// — the snapshot lives behind an Arc internally.
#[derive(Debug, Clone)]
pub struct AtlasState {
    snapshot: std::sync::Arc<ManifestSnapshot>,
}

impl AtlasState {
    /// Build the state from the seed list. F4.2 replaces this with a
    /// constructor that reads the live blob set.
    #[must_use]
    pub fn from_seed_list() -> Self {
        Self {
            snapshot: std::sync::Arc::new(ManifestSnapshot::from_seed_list()),
        }
    }
}

impl Default for AtlasState {
    fn default() -> Self {
        Self::from_seed_list()
    }
}

/// Build the axum router. Mount under no prefix; routes already
/// carry the `/atlas/...` prefix verbatim.
pub fn atlas_routes(state: AtlasState) -> Router {
    Router::new()
        .route("/atlas/manifest.json", get(handle_manifest))
        .route("/atlas/{slug}/survey", get(handle_query_stub))
        .route("/atlas/{slug}/symbols", get(handle_query_stub))
        .route("/atlas/{slug}/references", get(handle_query_stub))
        .route("/atlas/{slug}/read", get(handle_query_stub))
        .route("/atlas/{slug}/extract", get(handle_query_stub))
        .route("/atlas/{slug}/toc", get(handle_query_stub))
        .with_state(state)
}

async fn handle_manifest(State(state): State<AtlasState>) -> Json<ManifestSnapshot> {
    Json((*state.snapshot).clone())
}

/// JSON payload returned by the F2.6 v0 query stubs so clients can
/// branch on a stable reason tag.
#[derive(Debug, Serialize)]
struct NotIndexedYet {
    reason: &'static str,
    slug: String,
    /// Manifest URL the client should consult to know which slugs
    /// are accepted.
    manifest_url: &'static str,
}

async fn handle_query_stub(Path(slug): Path<String>) -> impl IntoResponse {
    let is_known = ATLAS_SEED_REPOS.iter().any(|r| r.slug == slug);
    let status = if is_known {
        StatusCode::SERVICE_UNAVAILABLE
    } else {
        StatusCode::NOT_FOUND
    };
    let body = NotIndexedYet {
        reason: if is_known { "atlas_not_indexed_yet" } else { "unknown_atlas_slug" },
        slug,
        manifest_url: "/atlas/manifest.json",
    };
    (status, Json(body))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::{to_bytes, Body};
    use axum::http::Request;
    use tower::ServiceExt as _;

    fn app() -> Router {
        atlas_routes(AtlasState::from_seed_list())
    }

    #[tokio::test]
    async fn manifest_returns_seed_list_snapshot() {
        let resp = app()
            .oneshot(Request::builder().uri("/atlas/manifest.json").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = to_bytes(resp.into_body(), 1_000_000).await.unwrap();
        let body_str = String::from_utf8_lossy(&body);
        assert!(body_str.contains("\"schema_version\":1"));
        assert!(body_str.contains("\"slug\":\"react\""));
        assert!(body_str.contains("\"slug\":\"tokio\""));
    }

    #[tokio::test]
    async fn known_slug_returns_503_not_indexed_yet() {
        let resp = app()
            .oneshot(Request::builder().uri("/atlas/react/survey?query=hooks").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
        let body = to_bytes(resp.into_body(), 8192).await.unwrap();
        let body_str = String::from_utf8_lossy(&body);
        assert!(body_str.contains("atlas_not_indexed_yet"), "{body_str}");
        assert!(body_str.contains("\"slug\":\"react\""), "{body_str}");
    }

    #[tokio::test]
    async fn unknown_slug_returns_404() {
        let resp = app()
            .oneshot(Request::builder().uri("/atlas/no-such-repo/survey").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
        let body = to_bytes(resp.into_body(), 8192).await.unwrap();
        let body_str = String::from_utf8_lossy(&body);
        assert!(body_str.contains("unknown_atlas_slug"), "{body_str}");
    }
}
