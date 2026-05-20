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
//! Token source depends on the host:
//!
//! - **Azure Container Apps** (the prod path here): ACA does **not**
//!   expose the `IaaS` IMDS endpoint at `169.254.169.254`. Instead it
//!   injects two env vars into every container that has a managed
//!   identity bound — `IDENTITY_ENDPOINT` (typically
//!   `http://localhost:42356/msi/token`) and `IDENTITY_HEADER` (a
//!   per-replica secret). The token call is then:
//!   `GET ${IDENTITY_ENDPOINT}?resource=https://management.azure.com/&api-version=2019-08-01`
//!   with the header `X-IDENTITY-HEADER: ${IDENTITY_HEADER}`. This is
//!   the same protocol App Service / Functions use.
//!   (`learn.microsoft.com/azure/container-apps/managed-identity#rest-endpoint-reference`.)
//!
//! - **VMSS / classic VM**: `GET http://169.254.169.254/metadata/identity/oauth2/token?api-version=2018-02-01&resource=https://management.azure.com/`
//!   with header `Metadata: true`. Kept as a fallback so the same
//!   binary still works if we ever run the trigger inside a VMSS
//!   sidecar.
//!
//! Either way the response is `{ access_token, expires_on, ... }` and
//! `expires_on` is an epoch-second string.
//!
//! After fetching the token:
//!
//! `POST {arm}/subscriptions/{sub}/resourceGroups/{rg}/providers/Microsoft.App/jobs/{name}/start?api-version=2026-01-01`
//! with `Authorization: Bearer <token>` and an empty JSON body.
//! ACA accepts 200/202; either is success.
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

/// IMDS api-version for the VMSS / `IaaS` endpoint at `169.254.169.254`.
/// `2018-02-01` is the stable surface and the version Azure SDK clients
/// pin to today; later versions exist but add no fields we read.
const VMSS_IMDS_API_VERSION: &str = "2018-02-01";

/// IMDS api-version for the ACA / App Service / Functions identity
/// endpoint (the one selected by `IDENTITY_ENDPOINT` env var).
/// `2019-08-01` is the value the Azure SDKs use against this surface;
/// older `2017-09-01` is documented but produces a different response
/// shape on some Azure properties — pinning to `2019-08-01` keeps the
/// `expires_on` field stable across hosts.
const ACA_IMDS_API_VERSION: &str = "2019-08-01";

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

/// IMDS protocol variant. Selected at construction time from the host
/// environment so the trigger can run on both ACA (the prod target) and
/// VMSS (the historical default that 169.254 documentation describes).
#[derive(Debug, Clone)]
pub enum ImdsAuth {
    /// Classic `IaaS` / VMSS IMDS at the well-known link-local address.
    /// Request shape: `GET <base>/metadata/identity/oauth2/token?api-version=2018-02-01&resource=<R>`
    /// with header `Metadata: true`.
    Vmss { base_url: String },
    /// ACA / App Service / Functions identity endpoint. Request shape:
    /// `GET <endpoint>?resource=<R>&api-version=2019-08-01` with header
    /// `X-IDENTITY-HEADER: <header_secret>`. ACA injects both fields as
    /// `IDENTITY_ENDPOINT` / `IDENTITY_HEADER` env vars when the
    /// container app has a managed identity bound.
    Aca {
        endpoint: String,
        header_secret: String,
    },
}

impl ImdsAuth {
    /// Auto-detect the right variant from process env. Prefers the ACA
    /// path when both `IDENTITY_ENDPOINT` and `IDENTITY_HEADER` resolve;
    /// otherwise returns the VMSS variant at the well-known link-local
    /// address.
    #[must_use]
    pub fn detect() -> Self {
        let read = |k: &str| -> Option<String> {
            std::env::var(k)
                .ok()
                .map(|s| s.trim().to_owned())
                .filter(|s| !s.is_empty())
        };
        match (read("IDENTITY_ENDPOINT"), read("IDENTITY_HEADER")) {
            (Some(endpoint), Some(header_secret)) => Self::Aca {
                endpoint: trim_trailing_slashes(endpoint),
                header_secret,
            },
            _ => Self::Vmss {
                base_url: "http://169.254.169.254".to_owned(),
            },
        }
    }

    fn token_url(&self) -> String {
        match self {
            Self::Vmss { base_url } => format!(
                "{base_url}/metadata/identity/oauth2/token?api-version={VMSS_IMDS_API_VERSION}&resource={ARM_RESOURCE}",
            ),
            Self::Aca { endpoint, .. } => format!(
                "{endpoint}?resource={ARM_RESOURCE}&api-version={ACA_IMDS_API_VERSION}",
            ),
        }
    }

    /// Brief shape for the Debug impl on the trigger — avoids leaking
    /// the header secret in logs.
    fn variant_name(&self) -> &'static str {
        match self {
            Self::Vmss { .. } => "vmss",
            Self::Aca { .. } => "aca",
        }
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
    imds: ImdsAuth,
    cache: Arc<Mutex<HashMap<String, CachedArmToken>>>,
}

impl std::fmt::Debug for AcaJobStartTrigger {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AcaJobStartTrigger")
            .field("subscription_id", &self.config.subscription_id)
            .field("resource_group", &self.config.resource_group)
            .field("job_name", &self.config.job_name)
            .field("arm_base_url", &self.arm_base_url)
            .field("imds_variant", &self.imds.variant_name())
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
    /// Production constructor. Auto-detects the IMDS protocol from the
    /// process env via [`ImdsAuth::detect`] — ACA pods take the
    /// `IDENTITY_ENDPOINT`/`IDENTITY_HEADER` path; everything else falls
    /// back to the `IaaS` IMDS endpoint at `169.254.169.254`.
    ///
    /// # Errors
    ///
    /// Returns [`JobStartError::Http`] on `reqwest::Client` build
    /// failure (e.g. a system-CA misconfiguration). Configuration
    /// validity is the caller's responsibility — pass a fully populated
    /// [`AcaJobStartConfig`].
    pub fn new(config: AcaJobStartConfig) -> Result<Self, JobStartError> {
        Self::with_endpoints(config, "https://management.azure.com", ImdsAuth::detect())
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
        imds: ImdsAuth,
    ) -> Result<Self, JobStartError> {
        let http = reqwest::Client::builder()
            .timeout(HTTP_TIMEOUT)
            .user_agent("ministr-cloud-aca-trigger/1 (+https://ministr.ai)")
            // Loopback IMDS must never go through an HTTP proxy — even
            // a misconfigured `HTTPS_PROXY=…` on the pod would route a
            // 169.254 metadata call to the proxy and produce a token
            // for *the proxy's* identity. ACA's IDENTITY_ENDPOINT is
            // also localhost-only, so the same `no_proxy` covers both.
            .no_proxy()
            .build()
            .map_err(|e| JobStartError::Http(format!("build http: {e}")))?;
        Ok(Self {
            config,
            http,
            arm_base_url: trim_trailing_slashes(arm_base_url.into()),
            imds,
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
        let url = self.imds.token_url();
        // Both protocols require an anti-SSRF header — `Metadata: true`
        // for VMSS, `X-IDENTITY-HEADER: <secret>` for ACA. The secret
        // is a per-replica value Azure injects; logging it would expose
        // it, so the header value never reaches a tracing macro here.
        let mut request = self.http.get(&url);
        match &self.imds {
            ImdsAuth::Vmss { .. } => {
                request = request.header("Metadata", "true");
            }
            ImdsAuth::Aca { header_secret, .. } => {
                request = request.header("X-IDENTITY-HEADER", header_secret);
            }
        }
        let resp = request
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

    /// Expected ACA `X-IDENTITY-HEADER` secret. Tests assert the
    /// trigger forwarded this exact value so a typo in the auth path
    /// surfaces as a test failure rather than a 401 in prod.
    const ACA_SECRET: &str = "test-secret-do-not-leak";

    #[derive(Debug, Clone, Default)]
    struct MockShared {
        arm_calls: Arc<AtomicUsize>,
        imds_calls: Arc<AtomicUsize>,
        /// VMSS: enforce `Metadata: true`. ACA: enforce
        /// `X-IDENTITY-HEADER: ACA_SECRET`. Mirrors what each Azure
        /// IMDS surface actually rejects.
        imds_expects_metadata_header: bool,
        imds_expects_aca_header: bool,
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
        if s.imds_expects_aca_header
            && headers
                .get("X-IDENTITY-HEADER")
                .is_none_or(|v| v != ACA_SECRET)
        {
            return (StatusCode::BAD_REQUEST, "missing X-IDENTITY-HEADER")
                .into_response();
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
            // VMSS IMDS path.
            .route("/metadata/identity/oauth2/token", get(imds_handler))
            // ACA IDENTITY_ENDPOINT path — different URL, same handler.
            // PHASE5 chunk 1 hotfix wires both so a single mock proves
            // both protocols.
            .route("/msi/token", get(imds_handler))
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

    fn vmss_auth(base: &str) -> ImdsAuth {
        ImdsAuth::Vmss {
            base_url: base.to_owned(),
        }
    }

    fn aca_auth(base: &str) -> ImdsAuth {
        ImdsAuth::Aca {
            endpoint: format!("{base}/msi/token"),
            header_secret: ACA_SECRET.to_owned(),
        }
    }

    #[tokio::test]
    async fn happy_path_round_trips_against_mock_vmss() {
        let shared = MockShared {
            arm_status: 202,
            arm_body: String::new(),
            imds_expects_metadata_header: true,
            ..MockShared::default()
        };
        let (base, _h) = spawn_mock(shared.clone()).await;
        let trig = AcaJobStartTrigger::with_endpoints(cfg(), &base, vmss_auth(&base)).unwrap();
        trig.start_indexer_job("corpus-abc").await.unwrap();
        assert_eq!(shared.imds_calls.load(Ordering::SeqCst), 1);
        assert_eq!(shared.arm_calls.load(Ordering::SeqCst), 1);

        // Second call: IMDS hit comes from cache, ARM still fires.
        trig.start_indexer_job("corpus-abc").await.unwrap();
        assert_eq!(shared.imds_calls.load(Ordering::SeqCst), 1, "cached");
        assert_eq!(shared.arm_calls.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn happy_path_round_trips_against_mock_aca() {
        // PHASE5 chunk 1 hotfix — ACA pods can't reach 169.254.169.254;
        // they use IDENTITY_ENDPOINT + X-IDENTITY-HEADER. This test
        // pins that the trigger sends the right URL + header on that
        // path so a typo would fail here, not in prod.
        let shared = MockShared {
            arm_status: 202,
            arm_body: String::new(),
            imds_expects_aca_header: true,
            ..MockShared::default()
        };
        let (base, _h) = spawn_mock(shared.clone()).await;
        let trig = AcaJobStartTrigger::with_endpoints(cfg(), &base, aca_auth(&base)).unwrap();
        trig.start_indexer_job("corpus-abc").await.unwrap();
        assert_eq!(shared.imds_calls.load(Ordering::SeqCst), 1);
        assert_eq!(shared.arm_calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn aca_path_omitting_header_fails_imds() {
        // Negative test: if the trigger forgot to set X-IDENTITY-HEADER
        // the mock returns 400. Acts as a regression guard so a future
        // refactor doesn't silently drop the header.
        let shared = MockShared {
            arm_status: 202,
            imds_expects_aca_header: true,
            ..MockShared::default()
        };
        let (base, _h) = spawn_mock(shared).await;
        // Construct the ACA variant but with a WRONG secret so the
        // mock's header check fails.
        let bad_auth = ImdsAuth::Aca {
            endpoint: format!("{base}/msi/token"),
            header_secret: "wrong-secret".into(),
        };
        let trig = AcaJobStartTrigger::with_endpoints(cfg(), &base, bad_auth).unwrap();
        let err = trig.start_indexer_job("c1").await.unwrap_err();
        match err {
            JobStartError::Imds(msg) => assert!(msg.contains("400"), "got {msg:?}"),
            other => panic!("wanted Imds, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn arm_4xx_surfaces_as_arm_error() {
        let shared = MockShared {
            arm_status: 403,
            arm_body: "{\"error\":{\"code\":\"Forbidden\"}}".into(),
            ..MockShared::default()
        };
        let (base, _h) = spawn_mock(shared).await;
        let trig = AcaJobStartTrigger::with_endpoints(cfg(), &base, vmss_auth(&base)).unwrap();
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
            AcaJobStartTrigger::with_endpoints(cfg(), &base, vmss_auth(&base)).unwrap(),
        );
        trig.start_job_for("c1").await.unwrap();
    }

    #[test]
    fn imds_auth_token_url_shapes() {
        // Sanity-check the URL constructors so a typo lands as a unit-
        // test failure rather than a production token-fetch error.
        let vmss = ImdsAuth::Vmss {
            base_url: "http://169.254.169.254".into(),
        };
        let url = vmss.token_url();
        assert!(url.contains("/metadata/identity/oauth2/token"));
        assert!(url.contains(&format!("api-version={VMSS_IMDS_API_VERSION}")));
        assert!(url.contains(ARM_RESOURCE));

        let aca = ImdsAuth::Aca {
            endpoint: "http://localhost:42356/msi/token".into(),
            header_secret: "s".into(),
        };
        let url = aca.token_url();
        assert!(url.starts_with("http://localhost:42356/msi/token?"));
        assert!(url.contains(&format!("api-version={ACA_IMDS_API_VERSION}")));
        assert!(url.contains(ARM_RESOURCE));
    }

    #[test]
    fn trim_trailing_slashes_works() {
        assert_eq!(trim_trailing_slashes("https://x/".into()), "https://x");
        assert_eq!(trim_trailing_slashes("https://x///".into()), "https://x");
        assert_eq!(trim_trailing_slashes("https://x".into()), "https://x");
    }
}
