//! PHASE6 chunk 1 — Azure `OpenAI`–backed [`Embedder`] for the cloud worker.
//!
//! Replaces the local fastembed/ONNX path on the cloud serve pod. The
//! local CLI (`ministr index`) continues to use [`FastEmbedder`]
//! unchanged — only the cloud worker swaps in this implementation when
//! `MINISTR_EMBEDDER_KIND=openai` (selector landed in PHASE6 chunk 2's
//! `WorkerLoop` wiring; this module is the building block).
//!
//! # Why move embedding off the pod
//!
//! PHASE5's first live demo OOM-killed the indexer Job at the first
//! batch: `[mem] after embedder.embed() rss=3762 MB delta=+3637 MB` on
//! a 4 GiB pod. `text-embedding-3-small` on Azure `OpenAI` returns vectors
//! over the network for $0.02/1M tokens, dropping the worker's memory
//! footprint by an order of magnitude. See `deploy/azure/PHASE6.md` for
//! the full diagnosis.
//!
//! # Auth flow
//!
//! Two paths, picked at construction time:
//!
//! - **API key** (simplest, recommended for getting started): the
//!   `api-key` HTTP header carries the resource's primary or secondary
//!   key. Read from `MINISTR_AZURE_OPENAI_API_KEY` at the wire-up site.
//! - **Managed Identity** (preferred for prod): a bearer token minted
//!   from the ACA `IDENTITY_ENDPOINT` for the resource
//!   `https://cognitiveservices.azure.com`. Cached with proactive
//!   evict. Same shape as [`crate::job_start::AcaJobStartTrigger`]'s
//!   `ImdsAuth::Aca` path — could be reused if either ever needs the
//!   common factor extracted.
//!
//! # Request shape
//!
//! ```text
//! POST {endpoint}/openai/deployments/{deployment}/embeddings?api-version=2024-10-21
//! Authorization: Bearer {mi_token}    (OR)   api-key: {api_key}
//! Content-Type: application/json
//!
//! { "input": ["text 1", "text 2", ...], "dimensions": 384 }
//! ```
//!
//! The `dimensions` parameter is Matryoshka truncation — `text-
//! embedding-3-*` models support requesting a smaller output than the
//! native 1536. We default to 384 to match the local fastembed model's
//! dimensionality so HNSW indexes stay cross-compatible. Operators who
//! want full-fidelity 1536-dim embeddings build the embedder with
//! [`Self::with_dimensions`] and accept that those indexes are not
//! query-compatible with the local 384-dim ones.
//!
//! # Sync over async
//!
//! The [`Embedder`] trait is sync. [`Self::embed`] uses an async
//! [`reqwest::Client`] internally, bridged via
//! [`tokio::task::block_in_place`] + [`tokio::runtime::Handle::block_on`].
//! Two reasons for async-over-blocking rather than
//! `reqwest::blocking::Client`:
//!
//! 1. `reqwest::blocking::Client` spins up its own tokio runtime and
//!    panics when dropped from inside an outer tokio runtime — and the
//!    serve binary holds the embedder Arc for the entire process
//!    lifetime, so the drop happens at `#[tokio::main]` shutdown,
//!    inside the runtime.
//! 2. `block_in_place` requires a multi-threaded runtime (which is
//!    what `serve` uses) but doesn't construct a second runtime; it
//!    just signals the scheduler to move other work off this worker
//!    thread while we block. Lower overhead and no drop landmines.
//!
//! The worker runs one ingestion at a time per replica (PHASE6 chunk
//! 2's `WorkerLoop` sets `concurrency=1`), so blocking the calling
//! tokio thread for ~500ms–2s per batch is acceptable: at worst one
//! worker thread is parked at a time per replica, and HTTP serve
//! happens on a different thread.

use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::embedding::Embedder;
use crate::error::IndexError;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use tracing::{debug, warn};

/// Azure `OpenAI` api-version for the embeddings endpoint. `2024-10-21`
/// is the first stable surface that documents the `dimensions`
/// parameter for `text-embedding-3-*` models; pinning here keeps the
/// request shape stable across Azure regional rollouts.
const AZURE_OPENAI_API_VERSION: &str = "2024-10-21";

/// HTTP timeout for the embeddings request. Generous — embeddings on
/// `text-embedding-3-small` typically return in <500ms for a 256-input
/// batch, but Azure occasionally throttles to ~5s on tail latency.
const HTTP_TIMEOUT: Duration = Duration::from_secs(30);

/// Default Matryoshka dimension. 384 matches the local
/// `all-MiniLM-L6-v2*` family, so HNSW indexes built by either path
/// stay cross-compatible. Override with [`OpenAiEmbedder::with_dimensions`]
/// for full-fidelity 1536-dim runs (separate, incompatible indexes).
pub const DEFAULT_DIMENSIONS: usize = 384;

/// Max times to retry a `429 Too Many Requests` before surfacing the
/// failure to the caller. PHASE6 chunk 4b post-deploy: anyhow's
/// ~466K-token burst against a (default-too-small) S0 deployment was
/// rejected on the first batch; honouring the `Retry-After` header
/// for up to ~2 minutes total recovers cleanly without breaking the
/// ingestion pipeline's rollback semantics.
const MAX_429_RETRIES: usize = 3;

/// Fallback `Retry-After` value when the header is missing or
/// unparseable. Azure returns `Retry-After: 60` in practice but
/// nothing in the spec mandates it.
const FALLBACK_RETRY_AFTER: Duration = Duration::from_secs(30);

/// Azure `OpenAI` cognitive-services scope for Microsoft Entra bearer
/// tokens. The trailing slash is required by IMDS.
const AZURE_COGNITIVE_SERVICES_RESOURCE: &str = "https://cognitiveservices.azure.com";

/// IMDS api-version for the ACA `IDENTITY_ENDPOINT` path.
const ACA_IMDS_API_VERSION: &str = "2019-08-01";

/// Proactive cache eviction window for the MI bearer token — same
/// rationale as the GitHub App token cache: a long-running batch must
/// never race the wall-clock expiry mid-call.
const CACHE_PROACTIVE_EVICT_SECS: u64 = 10 * 60;

/// Authentication mode for [`OpenAiEmbedder`].
#[derive(Debug, Clone)]
pub enum OpenAiAuth {
    /// `api-key` header carrying the resource's primary or secondary
    /// key. Simplest path; works without a managed identity setup.
    ApiKey(String),
    /// Bearer token minted from the ACA `IDENTITY_ENDPOINT`. The
    /// container app's MI must hold `Cognitive Services User` (or
    /// equivalent) on the Azure `OpenAI` resource.
    ManagedIdentity {
        /// Value of the `IDENTITY_ENDPOINT` env var that ACA injects.
        endpoint: String,
        /// Value of the `IDENTITY_HEADER` env var. Treated as a secret;
        /// never logged.
        header_secret: String,
    },
}

impl OpenAiAuth {
    /// Auto-detect the right variant. Prefers `ApiKey` when
    /// `MINISTR_AZURE_OPENAI_API_KEY` is set (simpler, faster cold
    /// start), then falls back to `ManagedIdentity` when ACA's env
    /// vars are present. Returns `None` if neither is configured.
    #[must_use]
    pub fn from_env() -> Option<Self> {
        let read = |k: &str| -> Option<String> {
            std::env::var(k)
                .ok()
                .map(|s| s.trim().to_owned())
                .filter(|s| !s.is_empty())
        };
        if let Some(key) = read("MINISTR_AZURE_OPENAI_API_KEY") {
            return Some(Self::ApiKey(key));
        }
        match (read("IDENTITY_ENDPOINT"), read("IDENTITY_HEADER")) {
            (Some(endpoint), Some(header_secret)) => Some(Self::ManagedIdentity {
                endpoint: trim_trailing_slashes(endpoint),
                header_secret,
            }),
            _ => None,
        }
    }

    fn variant_name(&self) -> &'static str {
        match self {
            Self::ApiKey(_) => "api_key",
            Self::ManagedIdentity { .. } => "managed_identity",
        }
    }
}

/// Static configuration for [`OpenAiEmbedder`]. Built once at startup
/// from env (`cmd_serve_http`) or a test fixture.
#[derive(Debug, Clone)]
pub struct OpenAiConfig {
    /// Resource base URL — e.g. `https://my-aoai.openai.azure.com`.
    /// Read from `MINISTR_AZURE_OPENAI_ENDPOINT`.
    pub endpoint: String,
    /// Deployment name (NOT the model name) — the operator-assigned
    /// label on the deployed model in Azure `OpenAI` Studio. Read from
    /// `MINISTR_AZURE_OPENAI_DEPLOYMENT`.
    pub deployment: String,
    /// Authentication source.
    pub auth: OpenAiAuth,
}

impl OpenAiConfig {
    /// Build from env. Returns `None` if any required field is missing,
    /// so the caller can fall back to local fastembed cleanly.
    #[must_use]
    pub fn from_env() -> Option<Self> {
        let read = |k: &str| -> Option<String> {
            std::env::var(k)
                .ok()
                .map(|s| s.trim().to_owned())
                .filter(|s| !s.is_empty())
        };
        Some(Self {
            endpoint: trim_trailing_slashes(read("MINISTR_AZURE_OPENAI_ENDPOINT")?),
            deployment: read("MINISTR_AZURE_OPENAI_DEPLOYMENT")?,
            auth: OpenAiAuth::from_env()?,
        })
    }
}

/// Azure `OpenAI`–backed [`Embedder`]. Constructs async HTTP requests
/// against the resource's `/embeddings` endpoint, deserialises the
/// response into `Vec<Vec<f32>>` matching the [`Embedder`] contract.
///
/// The sync [`Embedder::embed`] is implemented via `block_in_place` on
/// the current tokio runtime — see the module preamble for why.
#[derive(Clone)]
pub struct OpenAiEmbedder {
    config: OpenAiConfig,
    http: reqwest::Client,
    dimensions: usize,
    /// MI token cache (only used when `auth` is `ManagedIdentity`).
    token_cache: Arc<Mutex<Option<CachedToken>>>,
}

impl std::fmt::Debug for OpenAiEmbedder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OpenAiEmbedder")
            .field("endpoint", &self.config.endpoint)
            .field("deployment", &self.config.deployment)
            .field("auth", &self.config.auth.variant_name())
            .field("dimensions", &self.dimensions)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone)]
struct CachedToken {
    token: String,
    usable_until: u64,
}

impl OpenAiEmbedder {
    /// Build the embedder. Uses [`DEFAULT_DIMENSIONS`] (384) so the
    /// HNSW indexes stay cross-compatible with the local fastembed
    /// path.
    ///
    /// # Errors
    ///
    /// Returns [`IndexError::EmbeddingFailed`] on `reqwest::Client`
    /// build failure (system-CA misconfiguration etc.).
    pub fn new(config: OpenAiConfig) -> Result<Self, IndexError> {
        Self::with_dimensions(config, DEFAULT_DIMENSIONS)
    }

    /// Build with an explicit output dimension. `text-embedding-3-*`
    /// supports Matryoshka truncation; any value from 1 to the model's
    /// native 1536 is valid. Indexes built with different dimensions
    /// are NOT query-compatible — pin this and never change it for a
    /// live index.
    ///
    /// # Errors
    ///
    /// Same as [`Self::new`].
    pub fn with_dimensions(config: OpenAiConfig, dimensions: usize) -> Result<Self, IndexError> {
        let http = reqwest::Client::builder()
            .timeout(HTTP_TIMEOUT)
            .user_agent("ministr-cloud-openai-embedder/1 (+https://ministr.ai)")
            .build()
            .map_err(|e| IndexError::EmbeddingFailed {
                reason: format!("openai: build http: {e}"),
            })?;
        Ok(Self {
            config,
            http,
            dimensions,
            token_cache: Arc::new(Mutex::new(None)),
        })
    }

    fn embeddings_url(&self) -> String {
        format!(
            "{}/openai/deployments/{}/embeddings?api-version={}",
            self.config.endpoint, self.config.deployment, AZURE_OPENAI_API_VERSION,
        )
    }

    /// Mint or read-from-cache an MI bearer token for the Azure `OpenAI`
    /// resource. Only called when `auth` is `ManagedIdentity`.
    async fn mi_token(&self, endpoint: &str, header_secret: &str) -> Result<String, IndexError> {
        if let Some(cached) = self.cached_token() {
            debug!("openai mi token cache hit");
            return Ok(cached);
        }
        let url = format!(
            "{endpoint}?resource={AZURE_COGNITIVE_SERVICES_RESOURCE}&api-version={ACA_IMDS_API_VERSION}",
        );
        let resp = self
            .http
            .get(&url)
            .header("X-IDENTITY-HEADER", header_secret)
            .send()
            .await
            .map_err(|e| IndexError::EmbeddingFailed {
                reason: format!("openai mi imds: {e}"),
            })?;
        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(IndexError::EmbeddingFailed {
                reason: format!("openai mi imds: status {} body {body}", status.as_u16()),
            });
        }
        let parsed: ImdsTokenResponse =
            resp.json().await.map_err(|e| IndexError::EmbeddingFailed {
                reason: format!("openai mi imds parse: {e}"),
            })?;
        let usable_until = parsed
            .expires_on
            .as_deref()
            .and_then(|s| s.parse::<u64>().ok())
            .map_or_else(
                || epoch_now().saturating_add(3000),
                |epoch| epoch.saturating_sub(CACHE_PROACTIVE_EVICT_SECS),
            );
        *self.token_cache.lock() = Some(CachedToken {
            token: parsed.access_token.clone(),
            usable_until,
        });
        Ok(parsed.access_token)
    }

    fn cached_token(&self) -> Option<String> {
        let now = epoch_now();
        let mut guard = self.token_cache.lock();
        if let Some(entry) = guard.as_ref() {
            if now < entry.usable_until {
                return Some(entry.token.clone());
            }
            *guard = None;
        }
        None
    }

    /// Apply the right authentication header(s) for this auth mode.
    async fn apply_auth(
        &self,
        request: reqwest::RequestBuilder,
    ) -> Result<reqwest::RequestBuilder, IndexError> {
        match &self.config.auth {
            OpenAiAuth::ApiKey(key) => Ok(request.header("api-key", key)),
            OpenAiAuth::ManagedIdentity {
                endpoint,
                header_secret,
            } => {
                let token = self.mi_token(endpoint, header_secret).await?;
                Ok(request.bearer_auth(token))
            }
        }
    }

    /// Async core of [`Embedder::embed`]. Public so consumers already
    /// inside an async context (no current example, but a future
    /// async-aware Embedder trait would call this directly without the
    /// `block_in_place` bridge).
    ///
    /// # Errors
    ///
    /// See [`IndexError::EmbeddingFailed`] — see the trait impl for
    /// the full failure surface (HTTP, MI, batch-size, dim, parse).
    pub async fn embed_async(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, IndexError> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }
        let body = EmbeddingsRequest {
            input: texts,
            dimensions: self.dimensions,
        };

        // PHASE6 chunk 4b — honour `Retry-After` on 429 up to
        // MAX_429_RETRIES. Azure's S0 tier emits this with the actual
        // recommended wait (~60s for ratelimit-reached errors); other
        // 4xx surface immediately. The retry loop rebuilds the request
        // each iteration because reqwest's RequestBuilder isn't
        // Cloneable for body-bearing requests once `.json()` is
        // attached.
        let mut attempts = 0_usize;
        let parsed: EmbeddingsResponse = loop {
            let request = self
                .http
                .post(self.embeddings_url())
                .header("content-type", "application/json")
                .json(&body);
            let request = self.apply_auth(request).await?;
            let resp = request
                .send()
                .await
                .map_err(|e| IndexError::EmbeddingFailed {
                    reason: format!("openai post: {e}"),
                })?;
            let status = resp.status();
            if status.is_success() {
                break resp.json().await.map_err(|e| IndexError::EmbeddingFailed {
                    reason: format!("openai parse: {e}"),
                })?;
            }
            if status.as_u16() == 429 && attempts < MAX_429_RETRIES {
                let wait = retry_after_from_headers(resp.headers()).unwrap_or(FALLBACK_RETRY_AFTER);
                attempts += 1;
                warn!(
                    attempt = attempts,
                    max = MAX_429_RETRIES,
                    wait_secs = wait.as_secs(),
                    "openai 429 — backing off and retrying",
                );
                tokio::time::sleep(wait).await;
                continue;
            }
            let body = resp.text().await.unwrap_or_default();
            // Trim long error bodies for log triage; Azure `OpenAI`
            // returns structured { error: { code, message } } JSON.
            let trimmed = if body.len() > 512 {
                let mut s = body[..512].to_owned();
                s.push_str("...");
                s
            } else {
                body
            };
            return Err(IndexError::EmbeddingFailed {
                reason: format!("openai status {}: {trimmed}", status.as_u16()),
            });
        };
        // Azure returns embeddings in input-order when there's no
        // multi-batch reordering — but the spec also says responses
        // carry an `index` field for correctness. Sort by index to be
        // robust against future API changes.
        let mut data = parsed.data;
        data.sort_by_key(|d| d.index);
        let vectors: Vec<Vec<f32>> = data.into_iter().map(|d| d.embedding).collect();
        if vectors.len() != texts.len() {
            warn!(
                requested = texts.len(),
                returned = vectors.len(),
                "openai returned mismatched batch size",
            );
            return Err(IndexError::EmbeddingFailed {
                reason: format!(
                    "openai batch-size mismatch: requested {} got {}",
                    texts.len(),
                    vectors.len(),
                ),
            });
        }
        for (i, vec) in vectors.iter().enumerate() {
            if vec.len() != self.dimensions {
                return Err(IndexError::EmbeddingFailed {
                    reason: format!(
                        "openai dim mismatch at index {i}: expected {} got {}",
                        self.dimensions,
                        vec.len(),
                    ),
                });
            }
        }
        Ok(vectors)
    }
}

impl Embedder for OpenAiEmbedder {
    /// Sync bridge over the async core. Uses
    /// [`tokio::task::block_in_place`] + [`tokio::runtime::Handle::block_on`]
    /// to await `embed_async` without constructing a second runtime.
    /// Requires a multi-threaded tokio runtime — the serve binary
    /// uses the default `#[tokio::main]` flavor which provides one.
    fn embed(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, IndexError> {
        let handle =
            tokio::runtime::Handle::try_current().map_err(|_| IndexError::EmbeddingFailed {
                reason: "OpenAiEmbedder::embed called outside a tokio runtime".to_owned(),
            })?;
        tokio::task::block_in_place(|| handle.block_on(self.embed_async(texts)))
    }

    fn dimension(&self) -> usize {
        self.dimensions
    }
}

// --- wire types ------------------------------------------------------

#[derive(Debug, Serialize)]
struct EmbeddingsRequest<'a> {
    input: &'a [&'a str],
    dimensions: usize,
}

#[derive(Debug, Deserialize)]
struct EmbeddingsResponse {
    data: Vec<EmbeddingItem>,
}

#[derive(Debug, Deserialize)]
struct EmbeddingItem {
    embedding: Vec<f32>,
    index: usize,
}

#[derive(Debug, Deserialize)]
struct ImdsTokenResponse {
    access_token: String,
    #[serde(default)]
    expires_on: Option<String>,
}

// --- helpers ---------------------------------------------------------

fn epoch_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// Parse `Retry-After` from response headers. Per RFC 7231 the value
/// can be either an integer-seconds delta OR an HTTP-date. Azure
/// `OpenAI` emits the integer-seconds form on 429; we only parse that
/// shape and fall back to [`FALLBACK_RETRY_AFTER`] when the header is
/// missing, malformed, or in the HTTP-date form.
fn retry_after_from_headers(headers: &reqwest::header::HeaderMap) -> Option<Duration> {
    let raw = headers.get(reqwest::header::RETRY_AFTER)?.to_str().ok()?;
    let secs: u64 = raw.trim().parse().ok()?;
    Some(Duration::from_secs(secs))
}

fn trim_trailing_slashes(mut s: String) -> String {
    while s.ends_with('/') {
        s.pop();
    }
    s
}

// --- tests -----------------------------------------------------------

#[cfg(test)]
mod tests {
    //! Mock Azure `OpenAI` via axum, same pattern as
    //! `job_start::tests`. Two configurations exercised:
    //!
    //! 1. **API key auth** — the simpler path; `api-key` header carries
    //!    the secret. No IMDS sub-call.
    //! 2. **MI auth** — bearer token from a mock IMDS endpoint with
    //!    `X-IDENTITY-HEADER`.
    //!
    //! Both confirm: correct URL shape, correct headers, correct request
    //! body (input + dimensions), batch-order preservation, dim
    //! enforcement, and 4xx surfacing as `IndexError::EmbeddingFailed`.
    //!
    //! Tests exercise [`OpenAiEmbedder::embed_async`] directly so they
    //! stay fully inside the tokio runtime; the sync [`Embedder::embed`]
    //! is a thin `block_in_place` bridge over the same path and is
    //! covered separately by `embed_sync_bridge_works`.
    use super::*;
    use axum::{
        Json, Router,
        extract::{Path, Query, State},
        http::{HeaderMap, StatusCode},
        response::IntoResponse,
        routing::{get, post},
    };
    use serde::Serialize;
    use serde_json::Value;
    use std::collections::HashMap as StdHashMap;
    use std::sync::atomic::{AtomicUsize, Ordering};

    const API_KEY: &str = "test-key-do-not-leak";
    const IDENTITY_HEADER: &str = "test-mi-header";
    const MOCK_DIMENSION: usize = 384;

    #[derive(Debug, Clone, Default)]
    struct MockShared {
        embed_calls: Arc<AtomicUsize>,
        imds_calls: Arc<AtomicUsize>,
        embed_status: u16,
        embed_body: Option<Value>,
        expect_api_key: bool,
        expect_mi_header: bool,
        /// PHASE6 chunk 4b — when true, the FIRST embed call returns
        /// 429 with `Retry-After: 1`; subsequent calls return
        /// `embed_status` normally. Drives the
        /// `retries_429_then_succeeds` test.
        simulate_429_first_call: bool,
    }

    #[derive(Serialize)]
    struct EmbedBody {
        data: Vec<EmbedDatum>,
    }

    #[derive(Serialize)]
    struct EmbedDatum {
        embedding: Vec<f32>,
        index: usize,
    }

    #[derive(Serialize)]
    struct ImdsBody {
        access_token: String,
        expires_on: String,
    }

    async fn imds_handler(State(s): State<MockShared>, headers: HeaderMap) -> impl IntoResponse {
        s.imds_calls.fetch_add(1, Ordering::SeqCst);
        if headers
            .get("X-IDENTITY-HEADER")
            .is_none_or(|v| v != IDENTITY_HEADER)
        {
            return (StatusCode::BAD_REQUEST, "missing X-IDENTITY-HEADER").into_response();
        }
        let body = ImdsBody {
            access_token: "mock-mi-token".into(),
            expires_on: (epoch_now() + 24 * 60 * 60).to_string(),
        };
        (StatusCode::OK, Json(body)).into_response()
    }

    async fn embed_handler(
        State(s): State<MockShared>,
        Path((deployment,)): Path<(String,)>,
        Query(q): Query<StdHashMap<String, String>>,
        headers: HeaderMap,
        Json(body): Json<Value>,
    ) -> impl IntoResponse {
        let call_number = s.embed_calls.fetch_add(1, Ordering::SeqCst);
        // Simulate-429-first-call: on the very first invocation,
        // return 429 + Retry-After: 1 regardless of the test's
        // embed_status. Subsequent calls fall through to the normal
        // path. Used by retries_429_then_succeeds.
        if s.simulate_429_first_call && call_number == 0 {
            let mut resp = (
                StatusCode::TOO_MANY_REQUESTS,
                Json(serde_json::json!({
                    "error": { "code": "RateLimitReached", "message": "first-call retry" }
                })),
            )
                .into_response();
            // Retry-After: 0 keeps the test fast — production uses
            // Azure's real header value (~60s on a rate-limited S0
            // deployment). The retry-from-header parse path is the
            // same either way.
            resp.headers_mut()
                .insert("Retry-After", axum::http::HeaderValue::from_static("0"));
            return resp;
        }
        // Persistent-429 path (gives_up_after_max_429_retries): emit
        // Retry-After: 0 on every 429 so the retry loop spins
        // instantly through MAX_429_RETRIES rather than waiting
        // FALLBACK_RETRY_AFTER × N. Same parse path as production.
        if s.embed_status == 429 {
            let mut resp = (
                StatusCode::TOO_MANY_REQUESTS,
                Json(s.embed_body.clone().unwrap_or(serde_json::json!({}))),
            )
                .into_response();
            resp.headers_mut()
                .insert("Retry-After", axum::http::HeaderValue::from_static("0"));
            return resp;
        }
        assert_eq!(deployment, "embed-test-deployment");
        assert_eq!(
            q.get("api-version").map(String::as_str),
            Some(AZURE_OPENAI_API_VERSION),
        );

        if s.expect_api_key {
            let key = headers
                .get("api-key")
                .and_then(|v| v.to_str().ok())
                .unwrap_or_default();
            assert_eq!(key, API_KEY, "api-key header mismatch");
        }
        if s.expect_mi_header {
            let auth = headers
                .get("authorization")
                .and_then(|v| v.to_str().ok())
                .unwrap_or_default();
            assert_eq!(
                auth, "Bearer mock-mi-token",
                "Bearer token from IMDS not threaded into Authorization header",
            );
        }

        let dims = body
            .get("dimensions")
            .and_then(Value::as_u64)
            .expect("dimensions in body");
        assert_eq!(
            usize::try_from(dims).expect("dimensions fits usize"),
            MOCK_DIMENSION,
        );
        let inputs = body
            .get("input")
            .and_then(Value::as_array)
            .expect("input array");

        let status =
            StatusCode::from_u16(s.embed_status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
        if let Some(custom) = s.embed_body.clone() {
            return (status, Json(custom)).into_response();
        }
        // Default: synthesise one vector per input. Reverse the input
        // order in the response to confirm the embedder's sort-by-index
        // logic is doing real work.
        let data: Vec<EmbedDatum> = inputs
            .iter()
            .enumerate()
            .rev()
            .map(|(i, _)| EmbedDatum {
                embedding: vec![f32::from(u8::try_from(i & 0xff).unwrap_or(0)); MOCK_DIMENSION],
                index: i,
            })
            .collect();
        (status, Json(EmbedBody { data })).into_response()
    }

    async fn spawn_mock(shared: MockShared) -> (String, tokio::task::JoinHandle<()>) {
        let app = Router::new()
            .route(
                "/openai/deployments/{deployment}/embeddings",
                post(embed_handler),
            )
            .route("/msi/token", get(imds_handler))
            .with_state(shared);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        (format!("http://{addr}"), handle)
    }

    fn cfg_api_key(base: &str) -> OpenAiConfig {
        OpenAiConfig {
            endpoint: base.to_owned(),
            deployment: "embed-test-deployment".into(),
            auth: OpenAiAuth::ApiKey(API_KEY.into()),
        }
    }

    fn cfg_mi(base: &str) -> OpenAiConfig {
        OpenAiConfig {
            endpoint: base.to_owned(),
            deployment: "embed-test-deployment".into(),
            auth: OpenAiAuth::ManagedIdentity {
                endpoint: format!("{base}/msi/token"),
                header_secret: IDENTITY_HEADER.into(),
            },
        }
    }

    #[tokio::test]
    async fn round_trips_against_mock_with_api_key() {
        let shared = MockShared {
            embed_status: 200,
            expect_api_key: true,
            ..MockShared::default()
        };
        let (base, _h) = spawn_mock(shared.clone()).await;
        let embedder = OpenAiEmbedder::new(cfg_api_key(&base)).unwrap();

        let result = embedder
            .embed_async(&["alpha", "beta", "gamma"])
            .await
            .unwrap();

        assert_eq!(result.len(), 3, "one vector per input");
        for v in &result {
            assert_eq!(v.len(), MOCK_DIMENSION);
        }
        assert_eq!(shared.embed_calls.load(Ordering::SeqCst), 1);
        assert_eq!(shared.imds_calls.load(Ordering::SeqCst), 0);

        // Confirm sort-by-index undid the mock's reverse-order shuffle:
        // index 0 in the response was the LAST element the mock built,
        // so its embedding's value should be 0.0 (i & 0xff with i=0).
        let first_value = result[0][0];
        assert!(
            (first_value - 0.0).abs() < f32::EPSILON,
            "sort-by-index didn't reorder; first vec[0]={first_value}",
        );
    }

    #[tokio::test]
    async fn round_trips_against_mock_with_mi() {
        let shared = MockShared {
            embed_status: 200,
            expect_mi_header: true,
            ..MockShared::default()
        };
        let (base, _h) = spawn_mock(shared.clone()).await;
        let embedder = OpenAiEmbedder::new(cfg_mi(&base)).unwrap();

        let result = embedder.embed_async(&["one", "two"]).await.unwrap();

        assert_eq!(result.len(), 2);
        assert_eq!(shared.embed_calls.load(Ordering::SeqCst), 1);
        // IMDS should have been called once to mint the token.
        assert_eq!(shared.imds_calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn mi_token_is_cached_across_batches() {
        let shared = MockShared {
            embed_status: 200,
            expect_mi_header: true,
            ..MockShared::default()
        };
        let (base, _h) = spawn_mock(shared.clone()).await;
        let embedder = OpenAiEmbedder::new(cfg_mi(&base)).unwrap();

        embedder.embed_async(&["a"]).await.unwrap();
        embedder.embed_async(&["b"]).await.unwrap();

        assert_eq!(shared.embed_calls.load(Ordering::SeqCst), 2);
        // Critical: IMDS should fire ONCE, not twice. The internal
        // cache survives across embed_async() calls — proves cache works.
        assert_eq!(
            shared.imds_calls.load(Ordering::SeqCst),
            1,
            "MI token was not cached across batches",
        );
    }

    #[tokio::test]
    async fn http_4xx_surfaces_as_embedding_failed() {
        // Non-429 4xx surfaces immediately (no retry). 429 is exercised
        // separately by `retries_429_then_succeeds` /
        // `gives_up_after_max_429_retries` below.
        let shared = MockShared {
            embed_status: 400,
            embed_body: Some(serde_json::json!({
                "error": { "code": "BadRequest", "message": "malformed input" }
            })),
            expect_api_key: true,
            ..MockShared::default()
        };
        let (base, _h) = spawn_mock(shared).await;
        let embedder = OpenAiEmbedder::new(cfg_api_key(&base)).unwrap();

        let err = embedder.embed_async(&["x"]).await.unwrap_err();
        match err {
            IndexError::EmbeddingFailed { reason } => {
                assert!(reason.contains("400"), "got {reason:?}");
                assert!(reason.contains("malformed input"), "got {reason:?}");
            }
            other => panic!("wanted EmbeddingFailed, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn retries_429_then_succeeds() {
        // PHASE6 chunk 4b — anyhow's 466K-token burst hit S0's per-
        // deployment TPM cap and got rejected with 429 + Retry-After.
        // The embedder must honor the header and recover. This test
        // uses `start_paused` so the simulated sleep doesn't actually
        // block — tokio's mocked clock advances instantly when no
        // task is ready.
        //
        // Mock: first call returns 429 with `Retry-After: 1`, second
        // call returns 200. We pause time and let the retry's sleep
        // resolve immediately under the paused clock.
        let shared = MockShared {
            embed_status: 200, // becomes 200 after first 429 (toggled below)
            expect_api_key: true,
            simulate_429_first_call: true,
            ..MockShared::default()
        };
        let (base, _h) = spawn_mock(shared.clone()).await;
        let embedder = OpenAiEmbedder::new(cfg_api_key(&base)).unwrap();

        // Drive the future; let tokio fast-forward the retry sleep.
        let fut = embedder.embed_async(&["alpha"]);
        let result = tokio::time::timeout(Duration::from_secs(120), fut)
            .await
            .expect("retry should complete before timeout")
            .expect("retry should produce a successful vector");

        assert_eq!(result.len(), 1);
        // Two requests total: 429 then 200.
        assert_eq!(shared.embed_calls.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn gives_up_after_max_429_retries() {
        // Persistent 429s exhaust the retry budget and surface as a
        // structured EmbeddingFailed so the WorkerLoop marks the job
        // Failed (rather than retrying forever).
        let shared = MockShared {
            embed_status: 429,
            embed_body: Some(serde_json::json!({
                "error": { "code": "RateLimitReached", "message": "Try later" }
            })),
            expect_api_key: true,
            ..MockShared::default()
        };
        let (base, _h) = spawn_mock(shared.clone()).await;
        let embedder = OpenAiEmbedder::new(cfg_api_key(&base)).unwrap();

        let fut = embedder.embed_async(&["x"]);
        let err = tokio::time::timeout(Duration::from_secs(300), fut)
            .await
            .expect("retry budget should exhaust before outer timeout")
            .unwrap_err();
        match err {
            IndexError::EmbeddingFailed { reason } => {
                assert!(reason.contains("429"), "got {reason:?}");
            }
            other => panic!("wanted EmbeddingFailed, got {other:?}"),
        }
        // 1 initial + MAX_429_RETRIES = 4 total calls.
        assert_eq!(
            shared.embed_calls.load(Ordering::SeqCst),
            MAX_429_RETRIES + 1,
            "expected initial call + MAX_429_RETRIES retries",
        );
    }

    #[tokio::test]
    async fn batch_size_mismatch_surfaces_clearly() {
        // Mock returns ONE embedding for a TWO-input request — should
        // surface as a clear error not a silent truncation.
        let shared = MockShared {
            embed_status: 200,
            embed_body: Some(serde_json::json!({
                "data": [
                    { "embedding": vec![0.1_f32; MOCK_DIMENSION], "index": 0 }
                ]
            })),
            expect_api_key: true,
            ..MockShared::default()
        };
        let (base, _h) = spawn_mock(shared).await;
        let embedder = OpenAiEmbedder::new(cfg_api_key(&base)).unwrap();

        let err = embedder.embed_async(&["a", "b"]).await.unwrap_err();
        match err {
            IndexError::EmbeddingFailed { reason } => {
                assert!(reason.contains("mismatch"), "got {reason:?}");
            }
            other => panic!("wanted EmbeddingFailed, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn empty_input_is_a_no_op() {
        let embedder = OpenAiEmbedder::new(OpenAiConfig {
            endpoint: "http://unused".into(),
            deployment: "unused".into(),
            auth: OpenAiAuth::ApiKey("unused".into()),
        })
        .unwrap();
        let out = embedder.embed_async(&[]).await.unwrap();
        assert!(out.is_empty());
    }

    #[tokio::test]
    async fn embedder_reports_configured_dimension() {
        let embedder = OpenAiEmbedder::with_dimensions(
            OpenAiConfig {
                endpoint: "http://unused".into(),
                deployment: "unused".into(),
                auth: OpenAiAuth::ApiKey("unused".into()),
            },
            768,
        )
        .unwrap();
        assert_eq!(embedder.dimension(), 768);
    }

    #[test]
    fn trim_trailing_slashes_works() {
        assert_eq!(trim_trailing_slashes("https://x/".into()), "https://x");
        assert_eq!(trim_trailing_slashes("https://x///".into()), "https://x");
        assert_eq!(trim_trailing_slashes("https://x".into()), "https://x");
    }

    #[tokio::test]
    async fn dyn_trait_dispatch_compiles() {
        let embedder = OpenAiEmbedder::new(OpenAiConfig {
            endpoint: "http://unused".into(),
            deployment: "unused".into(),
            auth: OpenAiAuth::ApiKey("unused".into()),
        })
        .unwrap();
        let _dyn_embedder: Arc<dyn Embedder> = Arc::new(embedder);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn embed_sync_bridge_works() {
        // Confirms the sync Embedder::embed wrapper round-trips through
        // block_in_place + Handle::block_on without panicking. The
        // bridge ALSO requires multi-threaded runtime — single-thread
        // would panic at block_in_place; this test pins the contract.
        let shared = MockShared {
            embed_status: 200,
            expect_api_key: true,
            ..MockShared::default()
        };
        let (base, _h) = spawn_mock(shared.clone()).await;
        let embedder = OpenAiEmbedder::new(cfg_api_key(&base)).unwrap();
        let dyn_embedder: Arc<dyn Embedder> = Arc::new(embedder);

        // Call the sync trait method directly from inside this async
        // test. The block_in_place bridge moves the await off this
        // worker thread.
        let result = dyn_embedder.embed(&["alpha", "beta"]).unwrap();
        assert_eq!(result.len(), 2);
        assert_eq!(shared.embed_calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn embed_sync_bridge_errors_outside_runtime() {
        // Plain #[test], no tokio runtime active. The bridge should
        // surface a structured error instead of panicking.
        let embedder = OpenAiEmbedder::new(OpenAiConfig {
            endpoint: "http://unused".into(),
            deployment: "unused".into(),
            auth: OpenAiAuth::ApiKey("unused".into()),
        })
        .unwrap();
        let err = embedder.embed(&["x"]).unwrap_err();
        match err {
            IndexError::EmbeddingFailed { reason } => {
                assert!(reason.contains("outside a tokio runtime"), "got {reason:?}");
            }
            other => panic!("wanted EmbeddingFailed, got {other:?}"),
        }
    }
}
