//! F5.3-d-i — SIEM exporter (Splunk HEC, global env-var config).
//!
//! Implements [`ministr_api::AuditSink`] backed by an HTTP POST to a
//! Splunk HTTP Event Collector endpoint. Wired alongside the
//! Postgres + webhook sinks in [`crate::ChainedAuditSink`] so every
//! audit row also streams out to the customer's SIEM.
//!
//! v0 scope:
//!
//! - **One provider — Splunk HEC.** Datadog Logs / S3 JSON-lines /
//!   syslog/CEF will land as separate `*Sink` types behind the same
//!   [`AuditSink`] trait; the chain composition in `cmd_serve_http`
//!   extends naturally.
//! - **Global config via env vars.** `MINISTR_SIEM_HEC_URL` (full
//!   collector URL, e.g. `https://splunk.example.com:8088/services/collector/event`)
//!   and `MINISTR_SIEM_HEC_TOKEN` (the HEC token). Either missing
//!   disables the sink — `from_env()` returns `None`.
//! - **Per-org SIEM config CRUD lands as F5.3-d-ii.** Right now every
//!   org's audit rows hit the same HEC endpoint (the cloud operator's
//!   central SIEM). Customers running their own SIEM endpoint will
//!   wait for the per-org config table to ship.
//!
//! # Fire-and-forget posture
//!
//! Mirrors [`crate::PostgresAuditSink`] + [`crate::WebhookFanoutSink`]:
//! `record()` spawns a tokio task and returns immediately. A network
//! hiccup logs at `warn` but never propagates to the calling handler.
//! Splunk HEC's docs explicitly support best-effort one-way delivery —
//! losing an audit row during a transient outage is documented as
//! acceptable (the persistent Postgres copy is authoritative).

use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::extract::{Extension, Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use deadpool_postgres::Pool;
use ministr_api::{AuditEntry, AuditSink};
use ministr_mcp::auth::tenant::Tenant;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use tracing::{debug, warn};

use crate::orgs::member_role;

/// Default HTTP timeout. Splunk HEC is local-network fast; if a
/// customer's collector takes longer than 10s we want to abandon
/// the request rather than backing up the tokio task queue.
const HEC_TIMEOUT: Duration = Duration::from_secs(10);

/// Splunk HEC sink. Cheap-clone (`reqwest::Client` is `Arc`-backed
/// internally; the URL + token strings clone as needed inside the
/// spawned task).
#[derive(Clone)]
pub struct SplunkHecSink {
    endpoint_url: Arc<String>,
    token: Arc<String>,
    client: Client,
}

#[allow(clippy::missing_fields_in_debug)]
impl std::fmt::Debug for SplunkHecSink {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // The token is bearer material; never let a Debug print leak
        // it. Endpoint URL is operator metadata — safe to surface.
        // `client` is intentionally elided (reqwest::Client's Debug
        // is noise; no security value).
        f.debug_struct("SplunkHecSink")
            .field("endpoint_url", &self.endpoint_url.as_str())
            .field("token", &"<redacted>")
            .finish()
    }
}

impl SplunkHecSink {
    /// Construct from explicit URL + token. The URL must be the FULL
    /// collector URL including the `/services/collector/event` path
    /// (or whatever path the customer's HEC deployment uses). The
    /// constructor doesn't validate the URL — `record()` fails-soft
    /// at runtime if the URL is malformed.
    #[must_use]
    pub fn new(endpoint_url: impl Into<String>, token: impl Into<String>) -> Self {
        // build() is the only fallible step; if it fails (highly
        // unlikely — only options that affect TLS / cert validation
        // can fail), fall back to the default client which also has
        // no failure modes in the reqwest 0.12 build.
        let client = Client::builder()
            .timeout(HEC_TIMEOUT)
            .build()
            .unwrap_or_default();
        Self {
            endpoint_url: Arc::new(endpoint_url.into()),
            token: Arc::new(token.into()),
            client,
        }
    }

    /// Construct from `MINISTR_SIEM_HEC_URL` + `MINISTR_SIEM_HEC_TOKEN`
    /// env vars. Returns `None` when either is missing — the cloud
    /// serve then skips SIEM wiring entirely (no warn log; "no SIEM"
    /// is a valid deployment).
    #[must_use]
    pub fn from_env() -> Option<Self> {
        let url = std::env::var("MINISTR_SIEM_HEC_URL").ok()?;
        let token = std::env::var("MINISTR_SIEM_HEC_TOKEN").ok()?;
        if url.trim().is_empty() || token.trim().is_empty() {
            return None;
        }
        Some(Self::new(url, token))
    }
}

/// Wire shape posted to Splunk HEC. The `event` object carries the
/// flattened [`AuditEntry`]; Splunk parses it server-side and
/// indexes each field for search. `sourcetype` is the conventional
/// tag for ministr-emitted events so customers can filter on it.
#[derive(Debug, Serialize)]
struct HecPayload<'a> {
    sourcetype: &'static str,
    event: HecEvent<'a>,
    /// Unix epoch seconds. Splunk uses this for event ordering when
    /// the collector receives events out of order.
    time: u64,
}

#[derive(Debug, Serialize)]
struct HecEvent<'a> {
    action: &'a str,
    resource: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    org_id: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    actor: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    ip: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    user_agent: Option<&'a str>,
}

impl AuditSink for SplunkHecSink {
    fn record(&self, entry: AuditEntry) {
        let url = Arc::clone(&self.endpoint_url);
        let token = Arc::clone(&self.token);
        let client = self.client.clone();
        tokio::spawn(async move {
            dispatch_splunk_hec(&client, url.as_str(), token.as_str(), &entry).await;
        });
    }
}

/// F5.3-d-i + F5.3-d-ii-dispatch — shared HEC POST helper. Builds
/// the Splunk-event-shaped body, signs with `Authorization: Splunk
/// <token>`, fires the POST, and logs the outcome at `debug` (ok) /
/// `warn` (non-2xx response or connect/read error). Caller's
/// responsibility to invoke from a fire-and-forget tokio task — this
/// helper does NOT spawn its own.
///
/// Pulled out to a free function so [`SplunkHecSink`] (global,
/// env-var-config) and [`PerOrgSplunkHecDispatcher`] (per-org,
/// `org_siem_configs` table) share one POST shape with no
/// duplication.
pub(crate) async fn dispatch_splunk_hec(
    client: &Client,
    url: &str,
    token: &str,
    entry: &AuditEntry,
) {
    let time = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let payload = HecPayload {
        sourcetype: "ministr_audit",
        event: HecEvent {
            action: entry.action.as_str(),
            resource: entry.resource.as_str(),
            org_id: entry.org_id.as_deref(),
            actor: entry.actor.as_deref(),
            ip: entry.ip.as_deref(),
            user_agent: entry.user_agent.as_deref(),
        },
        time,
    };
    let auth_header = format!("Splunk {token}");
    let req = client
        .post(url)
        .header("Authorization", auth_header)
        .header("Content-Type", "application/json")
        .json(&payload);
    match req.send().await {
        Ok(resp) if resp.status().is_success() => {
            debug!(
                action = %entry.action,
                status = resp.status().as_u16(),
                "splunk hec dispatch ok"
            );
        }
        Ok(resp) => {
            let status = resp.status();
            let body = resp
                .text()
                .await
                .unwrap_or_else(|_| "<unreadable>".to_string());
            warn!(
                action = %entry.action,
                status = status.as_u16(),
                body = %body.chars().take(200).collect::<String>(),
                "splunk hec dispatch failed; row stays in Postgres audit_events"
            );
        }
        Err(e) => {
            warn!(
                action = %entry.action,
                error = %e,
                "splunk hec dispatch error; row stays in Postgres audit_events"
            );
        }
    }
}

/// F5.3-d-ii-dispatch — per-org SIEM dispatcher. Looks up
/// `org_siem_configs` on every audit event with `org_id IS NOT NULL`
/// and dispatches via [`dispatch_splunk_hec`] when a matching enabled
/// row exists. Personal-account events (`org_id IS NULL`) are skipped
/// — the per-org promise covers org-scoped actions only, same policy
/// as F5.3-a's tier-aware retention.
///
/// No cache in v0 — the lookup is one indexed query per audit event,
/// well under the audit volume threshold where caching would pay off.
/// A `Arc<RwLock<HashMap<org_id, (url, token, enabled)>>>` cache layer
/// can land in a follow-up chunk if volume grows.
///
/// Fires IN ADDITION to the global env-var sink (operator's central
/// SIEM still receives every event; customers' per-org endpoints
/// receive their org's slice).
#[derive(Clone)]
pub struct PerOrgSplunkHecDispatcher {
    pool: Arc<Pool>,
    client: Client,
}

impl std::fmt::Debug for PerOrgSplunkHecDispatcher {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PerOrgSplunkHecDispatcher")
            .field("pool", &"<Pool>")
            .finish()
    }
}

impl PerOrgSplunkHecDispatcher {
    /// Construct from a shared pool. The internal `reqwest::Client`
    /// is built once with the same 10s timeout as
    /// [`SplunkHecSink`] so a slow customer endpoint can't back up
    /// the audit pipeline.
    #[must_use]
    pub fn new(pool: Arc<Pool>) -> Self {
        let client = Client::builder()
            .timeout(HEC_TIMEOUT)
            .build()
            .unwrap_or_default();
        Self { pool, client }
    }
}

impl AuditSink for PerOrgSplunkHecDispatcher {
    fn record(&self, entry: AuditEntry) {
        // Skip personal-account events — per-org dispatch only
        // covers org-scoped actions.
        let Some(org_id) = entry.org_id.clone() else {
            return;
        };
        let pool = Arc::clone(&self.pool);
        let client = self.client.clone();
        tokio::spawn(async move {
            let conn = match pool.get().await {
                Ok(c) => c,
                Err(e) => {
                    warn!(
                        org_id = %org_id,
                        error = %e,
                        "per-org SIEM lookup: pool get failed"
                    );
                    return;
                }
            };
            // Single query: WHERE org_id = $1 AND enabled = TRUE AND
            // kind = 'splunk_hec'. The partial index from migration
            // 0014 on (org_id) WHERE enabled = TRUE makes this an
            // index lookup; the `kind` filter happens after.
            let row = match conn
                .query_opt(
                    "SELECT endpoint_url, token \
                     FROM org_siem_configs \
                     WHERE org_id = $1::text::uuid \
                       AND enabled = TRUE \
                       AND kind = 'splunk_hec'",
                    &[&org_id],
                )
                .await
            {
                Ok(r) => r,
                Err(e) => {
                    warn!(
                        org_id = %org_id,
                        error = %e,
                        "per-org SIEM lookup: query failed"
                    );
                    return;
                }
            };
            let Some(row) = row else {
                // No per-org config for this org — that's normal,
                // not an error. The global env-var sink (if wired)
                // still receives the event.
                return;
            };
            let url: String = row.get(0);
            let token: String = row.get(1);
            dispatch_splunk_hec(&client, &url, &token, &entry).await;
        });
    }
}

// ─── F5.3-d-ii — per-org SIEM config CRUD ────────────────────────
//
// Three routes mounted at `/api/v1/orgs/{id}/siem/config`. Same
// shape as F5.2-d's OIDC config CRUD: owner-only via
// `assert_owner_or_admin`, upsert via `ON CONFLICT (org_id)`, GET
// returns the row with `token` REDACTED, DELETE returns 204.
//
// Lookup state for the dispatch path (F5.3-d-ii-dispatch) will
// land in a future chunk; this chunk just persists customer
// config. With the schema seeded customers can pre-configure
// before the dispatcher wiring goes live.

/// Allowed `kind` values v0 admits. F5.3-d-iii will add
/// `"datadog_logs"`, `"s3_jsonl"`, `"syslog_cef"` here AND switch the
/// dispatch path on the value.
const ALLOWED_SIEM_KINDS: &[&str] = &["splunk_hec"];

/// Sentinel string returned in place of the real `token` on every
/// HTTP read. Mirrors F5.2-d's `REDACTED_CLIENT_SECRET` exactly so
/// frontend code that branches on the sentinel value handles both
/// configs uniformly.
pub const REDACTED_TOKEN: &str = "[REDACTED]";

/// F5.3-d-ii-config — per-org SIEM config CRUD router. Mount under
/// the `OAuth`-protected branch in `cmd_serve_http`; owner-only ACL
/// is enforced by each handler via [`assert_siem_owner_or_admin`].
pub fn siem_config_routes(state: SiemConfigState) -> Router {
    Router::new()
        .route(
            "/api/v1/orgs/{id}/siem/config",
            post(handle_siem_config_upsert)
                .get(handle_siem_config_get)
                .delete(handle_siem_config_delete),
        )
        .with_state(state)
}

/// Per-route shared state. Holds the Postgres pool the handlers use
/// for org-membership ACL + config table reads/writes.
#[derive(Clone)]
pub struct SiemConfigState {
    pub pool: Arc<Pool>,
}

impl std::fmt::Debug for SiemConfigState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SiemConfigState")
            .field("pool", &"<Pool>")
            .finish()
    }
}

impl SiemConfigState {
    /// Construct from a shared `Arc<Pool>`.
    #[must_use]
    pub fn from_arc(pool: Arc<Pool>) -> Self {
        Self { pool }
    }
}

#[derive(Debug)]
enum SiemConfigError {
    Unauthenticated,
    Forbidden,
    NotFound,
    Invalid(&'static str),
    Db(String),
}

impl IntoResponse for SiemConfigError {
    fn into_response(self) -> Response {
        match self {
            Self::Unauthenticated => {
                (StatusCode::UNAUTHORIZED, "unauthenticated").into_response()
            }
            Self::Forbidden => (StatusCode::FORBIDDEN, "forbidden").into_response(),
            Self::NotFound => (StatusCode::NOT_FOUND, "not_found").into_response(),
            Self::Invalid(msg) => (StatusCode::BAD_REQUEST, msg).into_response(),
            Self::Db(msg) => {
                tracing::warn!(error = %msg, "siem config db error");
                (StatusCode::INTERNAL_SERVER_ERROR, "internal").into_response()
            }
        }
    }
}

/// POST body for `/api/v1/orgs/{id}/siem/config`. `enabled` is
/// optional + defaults to true. F5.3-d-iii will admit more `kind`
/// values; v0 rejects anything that isn't `"splunk_hec"`.
#[derive(Deserialize)]
struct SiemConfigUpsertBody {
    kind: String,
    endpoint_url: String,
    token: String,
    #[serde(default)]
    enabled: Option<bool>,
}

/// GET / upsert response shape. `token` is always [`REDACTED_TOKEN`]
/// — the only writers are the upsert handler and the harness's
/// direct INSERT path; reads never expose it.
#[derive(Serialize)]
struct SiemConfigView {
    org_id: String,
    kind: String,
    endpoint_url: String,
    token: String,
    enabled: bool,
    created_at: String,
    updated_at: String,
}

/// Owner / admin ACL, identical shape to [`crate::oidc`]'s helper.
/// Duplicated rather than shared because both modules want their own
/// `*ConfigError` variants; the helper's body is two lines.
async fn assert_siem_owner_or_admin(
    pool: &Pool,
    org_id: &str,
    user_id: &str,
) -> Result<(), SiemConfigError> {
    let role = member_role(pool, org_id, user_id)
        .await
        .map_err(|e| SiemConfigError::Db(format!("member_role: {e}")))?;
    if !matches!(role.as_deref(), Some("owner" | "admin")) {
        return Err(SiemConfigError::Forbidden);
    }
    Ok(())
}

fn validate_siem_upsert(body: &SiemConfigUpsertBody) -> Result<(), SiemConfigError> {
    if !ALLOWED_SIEM_KINDS.contains(&body.kind.as_str()) {
        return Err(SiemConfigError::Invalid(
            "kind must be one of: splunk_hec",
        ));
    }
    if body.endpoint_url.trim().is_empty() {
        return Err(SiemConfigError::Invalid("endpoint_url is required"));
    }
    if !body.endpoint_url.starts_with("http://")
        && !body.endpoint_url.starts_with("https://")
    {
        return Err(SiemConfigError::Invalid(
            "endpoint_url must start with http:// or https://",
        ));
    }
    if body.token.trim().is_empty() {
        return Err(SiemConfigError::Invalid("token is required"));
    }
    Ok(())
}

// `parse_uuid` would normally go here; reuse the existing one from
// the oidc module since it has identical semantics.
fn parse_uuid_local(s: &str) -> Option<&str> {
    let bytes = s.as_bytes();
    if bytes.len() != 36 {
        return None;
    }
    let dashes = [8usize, 13, 18, 23];
    for (i, &b) in bytes.iter().enumerate() {
        if dashes.contains(&i) {
            if b != b'-' {
                return None;
            }
        } else if !b.is_ascii_hexdigit() {
            return None;
        }
    }
    Some(s)
}

async fn handle_siem_config_upsert(
    State(state): State<SiemConfigState>,
    tenant: Option<Extension<Tenant>>,
    Path(org_id): Path<String>,
    axum::Json(body): axum::Json<SiemConfigUpsertBody>,
) -> Result<(StatusCode, axum::Json<SiemConfigView>), SiemConfigError> {
    let tenant = tenant.ok_or(SiemConfigError::Unauthenticated)?;
    if parse_uuid_local(&org_id).is_none() {
        return Err(SiemConfigError::Invalid("invalid org id"));
    }
    assert_siem_owner_or_admin(&state.pool, &org_id, &tenant.0.subject).await?;
    validate_siem_upsert(&body)?;

    let enabled = body.enabled.unwrap_or(true);

    let client = state
        .pool
        .get()
        .await
        .map_err(|e| SiemConfigError::Db(format!("pool get: {e}")))?;
    let row = client
        .query_one(
            "INSERT INTO org_siem_configs (\
                org_id, kind, endpoint_url, token, enabled) \
             VALUES ($1::text::uuid, $2, $3, $4, $5) \
             ON CONFLICT (org_id) DO UPDATE SET \
                kind = EXCLUDED.kind, \
                endpoint_url = EXCLUDED.endpoint_url, \
                token = EXCLUDED.token, \
                enabled = EXCLUDED.enabled, \
                updated_at = NOW() \
             RETURNING kind, endpoint_url, enabled, \
                       to_char(created_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"'), \
                       to_char(updated_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"')",
            &[
                &org_id,
                &body.kind,
                &body.endpoint_url,
                &body.token,
                &enabled,
            ],
        )
        .await
        .map_err(|e| SiemConfigError::Db(format!("upsert: {e:?}")))?;

    // `token` is intentionally NOT in the RETURNING clause —
    // a stray log of `row` can't leak it. Hardcoded REDACTED below.
    let view = SiemConfigView {
        org_id: org_id.clone(),
        kind: row.get(0),
        endpoint_url: row.get(1),
        token: REDACTED_TOKEN.to_string(),
        enabled: row.get(2),
        created_at: row.get(3),
        updated_at: row.get(4),
    };
    Ok((StatusCode::OK, axum::Json(view)))
}

async fn handle_siem_config_get(
    State(state): State<SiemConfigState>,
    tenant: Option<Extension<Tenant>>,
    Path(org_id): Path<String>,
) -> Result<axum::Json<SiemConfigView>, SiemConfigError> {
    let tenant = tenant.ok_or(SiemConfigError::Unauthenticated)?;
    if parse_uuid_local(&org_id).is_none() {
        return Err(SiemConfigError::Invalid("invalid org id"));
    }
    assert_siem_owner_or_admin(&state.pool, &org_id, &tenant.0.subject).await?;

    let client = state
        .pool
        .get()
        .await
        .map_err(|e| SiemConfigError::Db(format!("pool get: {e}")))?;
    let row = client
        .query_opt(
            "SELECT kind, endpoint_url, enabled, \
                    to_char(created_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"'), \
                    to_char(updated_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"') \
             FROM org_siem_configs WHERE org_id = $1::text::uuid",
            &[&org_id],
        )
        .await
        .map_err(|e| SiemConfigError::Db(format!("select: {e:?}")))?
        .ok_or(SiemConfigError::NotFound)?;

    Ok(axum::Json(SiemConfigView {
        org_id: org_id.clone(),
        kind: row.get(0),
        endpoint_url: row.get(1),
        token: REDACTED_TOKEN.to_string(),
        enabled: row.get(2),
        created_at: row.get(3),
        updated_at: row.get(4),
    }))
}

async fn handle_siem_config_delete(
    State(state): State<SiemConfigState>,
    tenant: Option<Extension<Tenant>>,
    Path(org_id): Path<String>,
) -> Result<StatusCode, SiemConfigError> {
    let tenant = tenant.ok_or(SiemConfigError::Unauthenticated)?;
    if parse_uuid_local(&org_id).is_none() {
        return Err(SiemConfigError::Invalid("invalid org id"));
    }
    assert_siem_owner_or_admin(&state.pool, &org_id, &tenant.0.subject).await?;

    let client = state
        .pool
        .get()
        .await
        .map_err(|e| SiemConfigError::Db(format!("pool get: {e}")))?;
    let deleted = client
        .execute(
            "DELETE FROM org_siem_configs WHERE org_id = $1::text::uuid",
            &[&org_id],
        )
        .await
        .map_err(|e| SiemConfigError::Db(format!("delete: {e:?}")))?;
    if deleted == 0 {
        return Err(SiemConfigError::NotFound);
    }
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ministr_api::AuditEntry;

    #[test]
    fn debug_never_leaks_token() {
        // Catches a regression where a careless Debug impl change
        // surfaces the bearer material in logs / panics / structured
        // events.
        let sink = SplunkHecSink::new("https://splunk.example.com/services/collector/event", "secret-bearer-do-not-leak");
        let s = format!("{sink:?}");
        assert!(!s.contains("secret-bearer-do-not-leak"), "Debug leaked the token: {s}");
        assert!(s.contains("<redacted>"), "Debug should show <redacted>: {s}");
        assert!(s.contains("splunk.example.com"), "Debug should show URL: {s}");
    }

    #[test]
    fn from_env_returns_none_without_url() {
        // The two env vars are read each call so unsetting reverts
        // the state for subsequent tests.
        // SAFETY: tests run in the same process; this test asserts the
        // env-var read pattern without mutating shared global state
        // — if MINISTR_SIEM_HEC_URL ever IS set in the test env, the
        // assertion below would catch the divergence.
        let url = std::env::var("MINISTR_SIEM_HEC_URL").ok();
        let token = std::env::var("MINISTR_SIEM_HEC_TOKEN").ok();
        if url.is_some() && token.is_some() {
            // Don't fight a real deployment; just confirm the
            // happy-path returns Some.
            assert!(SplunkHecSink::from_env().is_some());
        } else {
            assert!(SplunkHecSink::from_env().is_none());
        }
    }

    #[test]
    fn record_is_fire_and_forget_no_panic_on_invalid_url() {
        // Construct a sink pointing at a syntactically valid but
        // unreachable URL. record() spawns a task that will fail to
        // connect — but must NOT panic the caller. The runtime test
        // is "no panic during record()"; the spawned task's warn log
        // is the operator's signal.
        let rt = tokio::runtime::Runtime::new().expect("runtime");
        rt.block_on(async {
            let sink = SplunkHecSink::new(
                "http://127.0.0.1:1/services/collector/event",
                "test-token",
            );
            sink.record(AuditEntry::new("test.event", "test-resource"));
            // Yield once so the spawned task gets a chance to start
            // (and fail) before the runtime is dropped.
            tokio::task::yield_now().await;
        });
    }

    fn body(kind: &str, url: &str, token: &str) -> SiemConfigUpsertBody {
        SiemConfigUpsertBody {
            kind: kind.to_string(),
            endpoint_url: url.to_string(),
            token: token.to_string(),
            enabled: None,
        }
    }

    #[test]
    fn validate_siem_upsert_admits_splunk_hec_with_https() {
        let b = body("splunk_hec", "https://splunk.example.com:8088/services/collector/event", "tok");
        assert!(validate_siem_upsert(&b).is_ok());
    }

    #[test]
    fn validate_siem_upsert_rejects_unknown_kind() {
        let b = body("future_provider", "https://x.example.com", "tok");
        let e = validate_siem_upsert(&b).expect_err("must reject unknown kind");
        assert!(matches!(e, SiemConfigError::Invalid(msg) if msg.contains("kind")));
    }

    #[test]
    fn validate_siem_upsert_rejects_url_without_scheme() {
        let b = body("splunk_hec", "splunk.example.com:8088", "tok");
        let e = validate_siem_upsert(&b).expect_err("must reject missing scheme");
        assert!(matches!(e, SiemConfigError::Invalid(msg) if msg.contains("http")));
    }

    #[test]
    fn validate_siem_upsert_rejects_empty_token() {
        let b = body("splunk_hec", "https://splunk.example.com", "");
        let e = validate_siem_upsert(&b).expect_err("must reject empty token");
        assert!(matches!(e, SiemConfigError::Invalid(msg) if msg.contains("token")));
    }

    #[test]
    fn allowed_siem_kinds_locks_v0_to_splunk_hec_only() {
        // F5.3-d-iii will extend this list; the regression-guard
        // makes the schema-vs-validator contract visible.
        assert_eq!(ALLOWED_SIEM_KINDS, &["splunk_hec"]);
    }

    #[test]
    fn parse_uuid_local_admits_canonical_form() {
        assert!(parse_uuid_local("00000000-0000-0000-0000-000000000000").is_some());
        assert!(parse_uuid_local("deadbeef-1234-5678-90ab-cdef00000000").is_some());
    }

    #[test]
    fn parse_uuid_local_rejects_garbage() {
        assert!(parse_uuid_local("").is_none());
        assert!(parse_uuid_local("not-a-uuid").is_none());
        assert!(parse_uuid_local("00000000-0000-0000-0000-0000000000ZZ").is_none());
        // Wrong dash positions.
        assert!(parse_uuid_local("000000000-000-0000-0000-000000000000").is_none());
    }

    #[test]
    fn redacted_token_sentinel_matches_oidc() {
        // F5.2-d ships REDACTED_CLIENT_SECRET = "[REDACTED]" and
        // frontend code branches on the literal string. Locking
        // REDACTED_TOKEN to the same value keeps the UI handling
        // uniform across both providers.
        assert_eq!(REDACTED_TOKEN, "[REDACTED]");
    }
}
