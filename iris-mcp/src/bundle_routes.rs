//! HTTP endpoints for serving index bundles over Streamable HTTP.
//!
//! When iris runs as a remote MCP server, these routes let clients download
//! pre-built index bundles or check their freshness without a full download.
//!
//! # Endpoints
//!
//! - `HEAD /bundles` — version metadata headers only (no body)
//! - `GET /bundles` — full bundle download with conditional GET (`If-None-Match`)
//! - `GET /bundles/manifest` — JSON manifest for lightweight staleness checks

use std::path::PathBuf;
use std::sync::Arc;

use axum::extract::State;
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::IntoResponse;
use axum::routing::get;
use axum::{Json, Router};
use tracing::instrument;

use iris_core::bundle::{
    self, BUNDLE_FORMAT_VERSION, BundleCorpusRoot, BundleManifest, compute_bundle_version,
};
use iris_core::index::{VectorIndex as _, VectorIndexLoad as _};
use iris_core::storage::{SqliteStorage, Storage};

/// Shared state for bundle HTTP endpoints.
#[derive(Clone)]
pub struct BundleState {
    /// Path to the corpus data directory (contains content.db + index/).
    pub corpus_dir: PathBuf,
    /// Embedding model name used for vectors in this corpus.
    pub model_name: String,
    /// Shared storage handle for reading corpus metadata.
    pub storage: Arc<SqliteStorage>,
}

/// Create an axum [`Router`] with bundle-serving endpoints.
///
/// Mount this at the application level — bundle endpoints are read-only
/// and typically public (not behind OAuth).
pub fn bundle_routes(state: BundleState) -> Router {
    Router::new()
        .route("/bundles", get(get_bundle).head(head_bundle))
        .route("/bundles/manifest", get(get_manifest))
        .with_state(state)
}

/// Build a [`BundleManifest`] from current corpus state.
async fn build_manifest(state: &BundleState) -> Result<BundleManifest, StatusCode> {
    let doc_count = state.storage.document_count().await.map_err(|e| {
        tracing::error!("failed to count documents: {e}");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;
    let roots = state.storage.list_corpus_roots().await.map_err(|e| {
        tracing::error!("failed to list corpus roots: {e}");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    let bundle_roots: Vec<BundleCorpusRoot> = roots
        .iter()
        .map(|r| BundleCorpusRoot {
            id: r.id.clone(),
            display_name: r.display_name.clone(),
            kind: r.kind.as_str().to_string(),
            commit_sha: r.commit_sha.clone(),
            branch: r.branch.clone(),
            repo_url: r.repo_url.clone(),
        })
        .collect();

    let source_commit = bundle_roots.iter().find_map(|r| r.commit_sha.clone());
    let bundle_version = Some(compute_bundle_version(&bundle_roots));

    let index_dir = state.corpus_dir.join("index");
    let (vector_count, dimension) = if index_dir.exists() {
        match iris_core::index::HnswIndex::load(&index_dir) {
            Ok(loaded) => (loaded.len(), loaded.dimension()),
            Err(_) => (0, 0),
        }
    } else {
        (0, 0)
    };

    Ok(BundleManifest {
        format_version: BUNDLE_FORMAT_VERSION,
        model_name: state.model_name.clone(),
        dimension,
        vector_count,
        document_count: doc_count,
        symbol_count: 0,
        corpus_roots: bundle_roots,
        created_at: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs(),
        bundle_version,
        source_commit,
    })
}

/// Append bundle version metadata headers to a [`HeaderMap`].
fn version_headers(headers: &mut HeaderMap, manifest: &BundleManifest) {
    if let Some(ref v) = manifest.bundle_version {
        let _ = headers.insert(
            "X-Iris-Bundle-Version",
            HeaderValue::from_str(v).unwrap_or_else(|_| HeaderValue::from_static("")),
        );
        // Use bundle_version as ETag for conditional GET.
        let etag = format!("\"{v}\"");
        let _ = headers.insert(
            header::ETAG,
            HeaderValue::from_str(&etag).unwrap_or_else(|_| HeaderValue::from_static("")),
        );
    }
    if let Some(ref sha) = manifest.source_commit {
        let _ = headers.insert(
            "X-Iris-Source-Commit",
            HeaderValue::from_str(sha).unwrap_or_else(|_| HeaderValue::from_static("")),
        );
    }
    let _ = headers.insert("X-Iris-Created-At", HeaderValue::from(manifest.created_at));
}

/// `HEAD /bundles` — version metadata headers without body.
#[instrument(skip_all)]
async fn head_bundle(State(state): State<BundleState>) -> Result<impl IntoResponse, StatusCode> {
    let manifest = build_manifest(&state).await?;
    let mut headers = HeaderMap::new();
    version_headers(&mut headers, &manifest);
    Ok((StatusCode::OK, headers))
}

/// `GET /bundles` — full bundle download with conditional GET support.
///
/// If the client sends `If-None-Match` matching the current `ETag`
/// (bundle version), returns `304 Not Modified` with no body.
#[instrument(skip_all)]
async fn get_bundle(
    State(state): State<BundleState>,
    request_headers: HeaderMap,
) -> Result<impl IntoResponse, StatusCode> {
    let db_path = state.corpus_dir.join("content.db");
    if !db_path.exists() {
        return Err(StatusCode::NOT_FOUND);
    }

    let manifest = build_manifest(&state).await?;

    // Conditional GET: if client's ETag matches, return 304.
    if let Some(if_none_match) = request_headers.get(header::IF_NONE_MATCH) {
        if let (Some(version), Ok(client_etag)) = (&manifest.bundle_version, if_none_match.to_str())
        {
            let expected = format!("\"{version}\"");
            if client_etag == expected {
                let mut headers = HeaderMap::new();
                version_headers(&mut headers, &manifest);
                return Ok((StatusCode::NOT_MODIFIED, headers, Vec::new()));
            }
        }
    }

    // Export the bundle to a temporary file.
    let temp_dir = tempfile::tempdir().map_err(|e| {
        tracing::error!("failed to create temp dir: {e}");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;
    let bundle_path = temp_dir.path().join("export.iris-index");

    bundle::export_bundle(&state.corpus_dir, &bundle_path, &manifest).map_err(|e| {
        tracing::error!("failed to export bundle: {e}");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    let body = tokio::fs::read(&bundle_path).await.map_err(|e| {
        tracing::error!("failed to read exported bundle: {e}");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    let mut headers = HeaderMap::new();
    version_headers(&mut headers, &manifest);
    let _ = headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/octet-stream"),
    );
    let _ = headers.insert(
        header::CONTENT_DISPOSITION,
        HeaderValue::from_static("attachment; filename=\"index.iris-index\""),
    );

    Ok((StatusCode::OK, headers, body))
}

/// `GET /bundles/manifest` — JSON manifest for lightweight staleness checks.
#[instrument(skip_all)]
async fn get_manifest(State(state): State<BundleState>) -> Result<impl IntoResponse, StatusCode> {
    let manifest = build_manifest(&state).await?;
    let mut headers = HeaderMap::new();
    version_headers(&mut headers, &manifest);
    Ok((StatusCode::OK, headers, Json(manifest)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_headers_populated() {
        let manifest = BundleManifest {
            format_version: BUNDLE_FORMAT_VERSION,
            model_name: "test".into(),
            dimension: 384,
            vector_count: 0,
            document_count: 0,
            symbol_count: 0,
            corpus_roots: vec![],
            created_at: 1_700_000_000,
            bundle_version: Some("abc123".into()),
            source_commit: Some("deadbeef".into()),
        };
        let mut headers = HeaderMap::new();
        version_headers(&mut headers, &manifest);

        assert_eq!(headers.get("X-Iris-Bundle-Version").unwrap(), "abc123");
        assert_eq!(headers.get("X-Iris-Source-Commit").unwrap(), "deadbeef");
        assert_eq!(headers.get(header::ETAG).unwrap(), "\"abc123\"");
        assert_eq!(headers.get("X-Iris-Created-At").unwrap(), "1700000000");
    }

    #[test]
    fn version_headers_none_fields() {
        let manifest = BundleManifest {
            format_version: BUNDLE_FORMAT_VERSION,
            model_name: "test".into(),
            dimension: 384,
            vector_count: 0,
            document_count: 0,
            symbol_count: 0,
            corpus_roots: vec![],
            created_at: 0,
            bundle_version: None,
            source_commit: None,
        };
        let mut headers = HeaderMap::new();
        version_headers(&mut headers, &manifest);

        assert!(headers.get("X-Iris-Bundle-Version").is_none());
        assert!(headers.get("X-Iris-Source-Commit").is_none());
        assert!(headers.get(header::ETAG).is_none());
        // created_at always present
        assert!(headers.get("X-Iris-Created-At").is_some());
    }
}
