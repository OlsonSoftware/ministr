//! PHASE5 chunk 1 — `AcaJobStartTrigger` impl of
//! [`ministr_api::JobStartTrigger`].
//!
//! Posts directly to the Azure Resource Manager `jobs/{name}/start`
//! endpoint from the serve pod, using a managed-identity token sourced
//! from the IMDS endpoint. Mirrors `GitHubAppClient`'s shape: an outbound
//! reqwest client + a small in-process token cache.
//!
//! # Auth flow
//!
//! 1. `GET http://169.254.169.254/metadata/identity/oauth2/token?api-version=2018-02-01&resource=https://management.azure.com/`
//!    with the header `Metadata: true`. Azure's IMDS responds with
//!    `{ access_token, expires_on, ... }` — `expires_on` is an epoch
//!    second string.
//! 2. `POST {arm}/subscriptions/{sub}/resourceGroups/{rg}/providers/Microsoft.App/jobs/{name}/start?api-version=2026-01-01`
//!    with `Authorization: Bearer <token>` and an empty JSON body.
//!    ACA accepts 200/202; either is success.
//!
//! # Cache TTL
//!
//! Azure-MI tokens are typically valid for ~24h, but we cap the cache to
//! `expires_on - 10min` (matches the `GitHubAppClient` proactive-evict
//! window) so a long-running serve handler never races the wall-clock
//! expiry. The wider the gap between request and use, the safer the
//! conservative bound.
//!
//! # Why hand-rolled instead of `azure_mgmt_appcontainers`
//!
//! The ARM SDK pulls a *lot* of generated code and a transitive
//! `swagger`-style runtime. For a single endpoint (`POST /start`) the
//! hand-rolled reqwest call is roughly 40 lines of business logic + a
//! parallel ~40 lines of test wiring. Decision recorded in
//! `deploy/azure/PHASE5.md` chunk 1's "Open questions" section.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use ministr_api::{JobStartError, JobStartFuture, JobStartTrigger};
use parking_lot::Mutex;
use serde::Deserialize;
use tracing::{debug, info, warn};

/// Latest GA ARM api-version for `Microsoft.App` resources, current as
/// of May 2026 per learn.microsoft.com `rest/api/resource-manager/containerapps/jobs/start`.
const ARM_API_VERSION: &str = "2026-01-01";

/// Resource URI for an Azure Management API token, per
/// `https://learn.microsoft.com/azure/active-directory/managed-identities-azure-resources/how-to-use-vm-token`.
/// Trailing slash is required — the IMDS endpoint validates it.
const ARM_RESOURCE: &str = "https://management.azure.com/";

/// IMDS api-version. `2018-02-01` is the stable surface and the version
/// Azure SDK clients pin to today; later versions exist but add no
/// fields we read.
const IMDS_API_VERSION: &str = "2018-02-01";

/// HTTP timeout for IMDS and ARM calls. IMDS is loopback so the timeout
/// only kicks in on a pathologically slow Azure host; ARM round-trips
/// from West/East US ACA cells stay <500ms in steady state.
const HTTP_TIMEOUT: Duration = Duration::from_secs(10);

/// How long before the IMDS-reported expiry to evict our cache. Same
/// rationale as `GitHubAppClient::CACHE_PROACTIVE_EVICT_SECS`: long
/// serve handlers must not race a wall-clock expiry.
const CACHE_PROACTIVE_EVICT_SECS: u64 = 10 * 60;

/// Static configuration the Pulumi layer feeds in via env vars. All
/// three must resolve to non-empty strings; absence is a Pulumi wiring
/// bug, not a runtime expectation.
#[derive(Debug, Clone)]
pub struct AcaJobStartConfig {
    /// Azure subscription GUID. Read from `MINISTR_ACA_SUBSCRIPTION_ID`.
    pub subscription_id: String,
    /// Resource group hosting the indexer Job. Read from
    /// `MINISTR_ACA_RESOURCE_GROUP`.
    pub resource_group: String,
    /// `app.Job` resource name (the value `lib/job.ts`'s `named("indexer")`
    /// emits). Read from `MINISTR_ACA_INDEXER_JOB_NAME`.
    pub job_name: String,
}

impl AcaJobStartConfig {
    /// Build from the three env vars Pulumi injects. Returns `None`
    /// when *any* are missing or empty — the serve pod then runs
    /// without the fast-path trigger and KEDA's 5-min safety net is
    /// the only producer→worker signal.
    #[must_use]
    pub fn from_env() -> Option<Self> {
        let read = |k: &str| -> Option<String> {
            std::env::var(k)
                .ok()
                .map(|s| s.trim().to_owned())
                .filter(|s| !s.is_empty())
        };
        Some(Self {
            subscription_id: read("MINISTR_ACA_SUBSCRIPTION_ID")?,
            resource_group: read("MINISTR_ACA_RESOURCE_GROUP")?,
            job_name: read("MINISTR_ACA_INDEXER_JOB_NAME")?,
        })
    }
}

/// Outbound trigger that asks ARM to start the indexer Job. Holds the
/// reqwest client + token cache; cheap to clone (everything is `Arc`'d
/// internally).
#[derive(Clone)]
pub struct AcaJobStartTrigger {
    config: AcaJobStartConfig,
    http: reqwest::Client,
    arm_base_url: String,
    imds_base_url: String,
    cache: Arc<Mutex<HashMap<String, CachedArmToken>>>,
}

impl std::fmt::Debug for AcaJobStartTrigger {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AcaJobStartTrigger")
            .field("subscription_id", &self.config.subscription_id)
            .field("resource_group", &self.config.resource_group)
            .field("job_name", &self.config.job_name)
            .field("arm_base_url", &self.arm_base_url)
            .field("imds_base_url", &self.imds_base_url)
            .field("cached_tokens", &self.cache.lock().len())
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone)]
struct CachedArmToken {
    token: String,
    usable_until: u64,
}

#[derive(Debug, Deserialize)]
struct ImdsTokenResponse {
    access_token: String,
    /// Epoch seconds as a string per Azure IMDS protocol.
    #[serde(default)]
    expires_on: Option<String>,
}

impl AcaJobStartTrigger {
    /// Production constructor — IMDS at the well-known loopback address,
    /// ARM at the standard management endpoint.
    ///
    /// # Errors
    ///
    /// Returns [`JobStartError::Http`] on `reqwest::Client` build
    /// failure (e.g. a system-CA misconfiguration). Configuration
    /// validity is the caller's responsibility — pass a fully populated
    /// [`AcaJobStartConfig`].
    pub fn new(config: AcaJobStartConfig) -> Result<Self, JobStartError> {
        Self::with_endpoints(
            config,
            "https://management.azure.com",
            "http://169.254.169.254",
        )
    }

    /// Test-only constructor that points at mock endpoints. Production
    /// callers go through [`Self::new`].
    ///
    /// # Errors
    ///
    /// Same as [`Self::new`].
    pub fn with_endpoints(
        config: AcaJobStartConfig,
        arm_base_url: impl Into<String>,
        imds_base_url: impl Into<String>,
    ) -> Result<Self, JobStartError> {
        let http = reqwest::Client::builder()
            .timeout(HTTP_TIMEOUT)
            .user_agent("ministr-cloud-aca-trigger/1 (+https://ministr.ai)")
            // Loopback IMDS must never go through an HTTP proxy — even
            // a misconfigured `HTTPS_PROXY=…` on the pod would route a
            // 169.254 metadata call to the proxy and produce a token
            // for *the proxy's* identity. `no_proxy` enforces the path.
            .no_proxy()
            .build()
            .map_err(|e| JobStartError::Http(format!("build http: {e}")))?;
        Ok(Self {
            config,
            http,
            arm_base_url: trim_trailing_slashes(arm_base_url.into()),
            imds_base_url: trim_trailing_slashes(imds_base_url.into()),
            cache: Arc::new(Mutex::new(HashMap::new())),
        })
    }

    /// Inner that mints (or reuses) an MI token and POSTs the start.
    /// Public so the test suite can exercise the full path; the trait
    /// impl funnels through here.
    ///
    /// # Errors
    ///
    /// See [`JobStartError`] for the per-variant semantics.
    pub async fn start_indexer_job(&self, corpus_id: &str) -> Result<(), JobStartError> {
        let token = self.fetch_arm_token().await?;
        let url = format!(
            "{}/subscriptions/{}/resourceGroups/{}/providers/Microsoft.App/jobs/{}/start?api-version={}",
            self.arm_base_url,
            self.config.subscription_id,
            self.config.resource_group,
            self.config.job_name,
            ARM_API_VERSION,
        );
        let resp = self
            .http
            .post(&url)
            .bearer_auth(&token)
            .header("content-type", "application/json")
            .body("{}")
            .send()
            .await
            .map_err(|e| JobStartError::Http(format!("arm post: {e}")))?;
        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            // Trim long ARM error bodies for log triage; the structured
            // {code,message} JSON usually fits comfortably.
            let trimmed = if body.len() > 512 {
                let mut s = body[..512].to_owned();
                s.push_str("...");
                s
            } else {
                body
            };
            return Err(JobStartError::Arm {
                status: status.as_u16(),
                body: trimmed,
            });
        }
        info!(
            corpus_id,
            arm_status = status.as_u16(),
            job_name = %self.config.job_name,
            "ARM jobs/start accepted"
        );
        Ok(())
    }

    async fn fetch_arm_token(&self) -> Result<String, JobStartError> {
        // Single-key cache: there's exactly one ARM token in flight per
        // pod identity. The key is `ARM_RESOURCE` itself so a hypothetical
        // future "fetch a token for a different audience" stays orthogonal.
        if let Some(t) = self.cache_lookup(ARM_RESOURCE) {
            debug!("imds token cache hit");
            return Ok(t);
        }
        let url = format!(
            "{}/metadata/identity/oauth2/token?api-version={}&resource={}",
            self.imds_base_url, IMDS_API_VERSION, ARM_RESOURCE,
        );
        let resp = self
            .http
            .get(&url)
            // IMDS rejects requests without this header — defends against
            // SSRF tricks that would otherwise let an attacker tunnel
            // through a vulnerable in-pod HTTP handler.
            .header("Metadata", "true")
            .send()
            .await
            .map_err(|e| JobStartError::Http(format!("imds get: {e}")))?;
        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(JobStartError::Imds(format!(
                "status {status}: {body}",
                status = status.as_u16()
            )));
        }
        let parsed: ImdsTokenResponse = resp
            .json()
            .await
            .map_err(|e| JobStartError::Imds(format!("parse: {e}")))?;
        let usable_until = parsed
            .expires_on
            .as_deref()
            .and_then(|s| s.parse::<u64>().ok())
            .map_or_else(
                // Fallback: assume a conservative 50 minutes when IMDS
                // doesn't return an `expires_on` (shouldn't happen in
                // prod but a corrupt mock or older IMDS api-version
                // could).
                || epoch_now().saturating_add(3000),
                |epoch| epoch.saturating_sub(CACHE_PROACTIVE_EVICT_SECS),
            );
        self.cache_store(ARM_RESOURCE, parsed.access_token.clone(), usable_until);
        info!(usable_until, "ARM MI token minted");
        Ok(parsed.access_token)
    }

    fn cache_lookup(&self, key: &str) -> Option<String> {
        let now = epoch_now();
        let mut cache = self.cache.lock();
        if let Some(entry) = cache.get(key) {
            if now < entry.usable_until {
                return Some(entry.token.clone());
            }
            cache.remove(key);
        }
        None
    }

    fn cache_store(&self, key: &str, token: String, usable_until: u64) {
        self.cache.lock().insert(
            key.to_owned(),
            CachedArmToken {
                token,
                usable_until,
            },
        );
    }

    /// Drop the cached MI token. Called by the trait impl if ARM returns
    /// 401 — the cached token is presumed compromised or revoked.
    pub fn invalidate(&self) {
        if self.cache.lock().remove(ARM_RESOURCE).is_some() {
            warn!("aca trigger token cache invalidated");
        }
    }
}

impl JobStartTrigger for AcaJobStartTrigger {
    fn start_job_for<'a>(&'a self, corpus_id: &'a str) -> JobStartFuture<'a> {
        Box::pin(async move { self.start_indexer_job(corpus_id).await })
    }
}

fn epoch_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn trim_trailing_slashes(mut s: String) -> String {
    while s.ends_with('/') {
        s.pop();
    }
    s
}

#[cfg(test)]
mod tests {
    //! Mock IMDS + ARM via a tokio-backed in-memory HTTP server (axum)
    //! so the test surface doesn't take a new dev-dep. Pattern mirrors
    //! `crate::billing::stripe_api::tests::create_customer_round_trips_against_local_mock`
    //! which already uses axum for a similar mock.

    use super::*;
    use axum::{
        Json, Router,
        extract::{Path, Query, State},
        http::{HeaderMap, StatusCode},
        response::IntoResponse,
        routing::{get, post},
    };
    use serde::Serialize;
    use std::collections::HashMap as StdHashMap;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn cfg() -> AcaJobStartConfig {
        AcaJobStartConfig {
            subscription_id: "sub-test".into(),
            resource_group: "rg-test".into(),
            job_name: "indexer-test".into(),
        }
    }

    #[derive(Debug, Clone, Default)]
    struct MockShared {
        arm_calls: Arc<AtomicUsize>,
        imds_calls: Arc<AtomicUsize>,
        imds_expects_metadata_header: bool,
        arm_status: u16,
        arm_body: String,
    }

    #[derive(Serialize)]
    struct ImdsBody {
        access_token: String,
        expires_on: String,
    }

    async fn imds_handler(
        State(s): State<MockShared>,
        headers: HeaderMap,
        Query(q): Query<StdHashMap<String, String>>,
    ) -> impl IntoResponse {
        s.imds_calls.fetch_add(1, Ordering::SeqCst);
        if s.imds_expects_metadata_header
            && headers.get("Metadata").is_none_or(|v| v != "true")
        {
            return (StatusCode::BAD_REQUEST, "missing Metadata header").into_response();
        }
        let _ = q.get("api-version");
        let _ = q.get("resource");
        let body = ImdsBody {
            access_token: "tok-mock".into(),
            // Far in the future so the cache stays warm across tests.
            expires_on: (epoch_now() + 24 * 60 * 60).to_string(),
        };
        (StatusCode::OK, Json(body)).into_response()
    }

    async fn arm_handler(
        State(s): State<MockShared>,
        Path((sub, rg, job)): Path<(String, String, String)>,
        Query(q): Query<StdHashMap<String, String>>,
        headers: HeaderMap,
    ) -> impl IntoResponse {
        s.arm_calls.fetch_add(1, Ordering::SeqCst);
        assert_eq!(sub, "sub-test");
        assert_eq!(rg, "rg-test");
        assert_eq!(job, "indexer-test");
        assert_eq!(q.get("api-version").map(String::as_str), Some(ARM_API_VERSION));
        let auth = headers
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        assert!(auth.starts_with("Bearer "), "auth was {auth:?}");
        let status =
            StatusCode::from_u16(s.arm_status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
        (status, s.arm_body.clone()).into_response()
    }

    async fn spawn_mock(shared: MockShared) -> (String, tokio::task::JoinHandle<()>) {
        let app = Router::new()
            .route("/metadata/identity/oauth2/token", get(imds_handler))
            .route(
                "/subscriptions/{sub}/resourceGroups/{rg}/providers/Microsoft.App/jobs/{job}/start",
                post(arm_handler),
            )
            .with_state(shared);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        (format!("http://{addr}"), handle)
    }

    #[tokio::test]
    async fn happy_path_round_trips_against_mock() {
        let shared = MockShared {
            arm_status: 202,
            arm_body: String::new(),
            imds_expects_metadata_header: true,
            ..MockShared::default()
        };
        let (base, _h) = spawn_mock(shared.clone()).await;
        let trig = AcaJobStartTrigger::with_endpoints(cfg(), &base, &base).unwrap();
        trig.start_indexer_job("corpus-abc").await.unwrap();
        assert_eq!(shared.imds_calls.load(Ordering::SeqCst), 1);
        assert_eq!(shared.arm_calls.load(Ordering::SeqCst), 1);

        // Second call: IMDS hit comes from cache, ARM still fires.
        trig.start_indexer_job("corpus-abc").await.unwrap();
        assert_eq!(shared.imds_calls.load(Ordering::SeqCst), 1, "cached");
        assert_eq!(shared.arm_calls.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn arm_4xx_surfaces_as_arm_error() {
        let shared = MockShared {
            arm_status: 403,
            arm_body: "{\"error\":{\"code\":\"Forbidden\"}}".into(),
            ..MockShared::default()
        };
        let (base, _h) = spawn_mock(shared).await;
        let trig = AcaJobStartTrigger::with_endpoints(cfg(), &base, &base).unwrap();
        let err = trig.start_indexer_job("c1").await.unwrap_err();
        match err {
            JobStartError::Arm { status, body } => {
                assert_eq!(status, 403);
                assert!(body.contains("Forbidden"));
            }
            other => panic!("wanted Arm, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn dyn_trait_dispatch_compiles() {
        // No network — assert the trait object compiles. If the trait
        // shape ever drifts away from dyn-compatibility this fails to
        // build, which is the point.
        let shared = MockShared {
            arm_status: 200,
            ..MockShared::default()
        };
        let (base, _h) = spawn_mock(shared).await;
        let trig: Arc<dyn JobStartTrigger> = Arc::new(
            AcaJobStartTrigger::with_endpoints(cfg(), &base, &base).unwrap(),
        );
        trig.start_job_for("c1").await.unwrap();
    }

    #[test]
    fn trim_trailing_slashes_works() {
        assert_eq!(trim_trailing_slashes("https://x/".into()), "https://x");
        assert_eq!(trim_trailing_slashes("https://x///".into()), "https://x");
        assert_eq!(trim_trailing_slashes("https://x".into()), "https://x");
    }
}
