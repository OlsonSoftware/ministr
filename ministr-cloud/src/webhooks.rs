//! F3.5a — outbound webhook subscriptions.
//!
//! Org admins register an HTTPS endpoint + event filter; the cloud
//! POSTs HMAC-SHA256-signed JSON to that endpoint whenever a matching
//! event fires. v0 ships:
//!
//! - `webhook_subscriptions` table + repo helpers (create / list /
//!   delete by id).
//! - `WebhookDispatcher` — outbound POST with 3 attempts and
//!   exponential backoff. Signs the body as
//!   `X-Ministr-Signature: sha256=<hex>` with a paired
//!   `X-Ministr-Timestamp: <unix_ts>` header. Receivers verify by
//!   recomputing `HMAC-SHA256(secret, ts + "." + body)` (Stripe-style
//!   replay-defeating construction — confirmed by the F1.5 inbound
//!   verifier in `billing::stripe`).
//! - Axum routes `POST /api/v1/orgs/{id}/webhooks`,
//!   `GET /api/v1/orgs/{id}/webhooks`,
//!   `DELETE /api/v1/orgs/{id}/webhooks/{wid}`,
//!   `POST /api/v1/orgs/{id}/webhooks/{wid}/test` (synthetic delivery).
//!
//! F3.5b adds the audit-feed `ChainedAuditSink` that fires webhooks on
//! real audit events plus the management UI in the Tauri panel.
//!
//! # Token discipline
//!
//! The signing secret is generated server-side, returned to the
//! caller exactly ONCE in the create response, and stored in
//! plaintext on the DB. (Hashing it would prevent the cloud from
//! signing outbound payloads since the cloud holds the only legitimate
//! signer.) The list endpoint never returns the secret.

use std::fmt::Write as _;
use std::sync::Arc;
use std::time::Duration;

use axum::Json;
use axum::Router;
use axum::extract::{Extension, Path, State};
use axum::http::StatusCode;
use axum::routing::{delete, post};
use deadpool_postgres::Pool;
use getrandom::fill as getrandom_fill;
use hmac::{Hmac, Mac};
use ministr_api::{AuditEntry, AuditSink};
use ministr_mcp::auth::tenant::Tenant;
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use tracing::{debug, warn};

use crate::orgs::member_role;

/// Random-bytes for the signing secret. 32 bytes → 256-bit entropy,
/// matching the F3.4a / F3.1b-i token convention.
const SECRET_BYTES: usize = 32;

/// Default delivery deadline per attempt. Three attempts at this
/// timeout each = ~30s upper bound for a stuck receiver.
const ATTEMPT_TIMEOUT: Duration = Duration::from_secs(10);

/// Inter-attempt backoff sequence. v0 uses 0/5/30s — the first retry
/// is fast (transient network blip), the second waits longer (the
/// receiver is genuinely down). All three attempts share a single
/// outbound request lifecycle from the dispatcher's perspective.
const RETRY_BACKOFF: &[Duration] = &[
    Duration::from_secs(0),
    Duration::from_secs(5),
    Duration::from_secs(30),
];

// ── Types ──────────────────────────────────────────────────────────────────

/// Errors surfaced by the webhooks module.
#[derive(Debug, thiserror::Error)]
pub enum WebhookError {
    /// Pool acquisition failed.
    #[error("get connection: {0}")]
    GetConn(String),
    /// SQL error.
    #[error("sql: {0}")]
    Sql(String),
    /// Validation failure surfaced as 400.
    #[error("invalid: {0}")]
    Invalid(&'static str),
}

/// One subscription row, shaped for the list response.
/// `secret` is intentionally absent — only the one-time create
/// response carries it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebhookSubscription {
    pub id: String,
    pub org_id: String,
    pub url: String,
    pub event_filter: String,
    pub created_by: String,
    /// ISO-8601 UTC.
    pub created_at: String,
    /// ISO-8601 UTC of the most-recent successful delivery; `None`
    /// when the subscription has never fired.
    pub last_delivered_at: Option<String>,
}

/// One subscription **with** secret — only returned by the create
/// endpoint. After this response the cloud holds the only copy of
/// the secret (used for HMAC signing).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreatedSubscription {
    #[serde(flatten)]
    pub subscription: WebhookSubscription,
    /// The HMAC signing secret. Mirror this on the receiver to
    /// verify inbound signatures. Pre-shared symmetric key; rotate
    /// by deleting + re-creating.
    pub secret: String,
}

// ── Repo ───────────────────────────────────────────────────────────────────

/// Mint a new subscription. Generates a fresh 32-byte secret;
/// returns `CreatedSubscription` carrying the secret exactly once.
///
/// # Errors
///
/// [`WebhookError::GetConn`] / [`WebhookError::Sql`] on DB issues.
pub async fn create_subscription(
    pool: &Pool,
    org_id: &str,
    url: &str,
    event_filter: &str,
    created_by: &str,
) -> Result<CreatedSubscription, WebhookError> {
    let secret = mint_secret();
    let client = pool
        .get()
        .await
        .map_err(|e| WebhookError::GetConn(format!("create_subscription: {e}")))?;
    let row = client
        .query_one(
            "INSERT INTO webhook_subscriptions
               (org_id, url, secret, event_filter, created_by)
             VALUES ($1::uuid, $2, $3, $4, $5::uuid)
             RETURNING
               id::text          AS id_text,
               org_id::text      AS org_id_text,
               url,
               event_filter,
               created_by::text  AS created_by_text,
               to_char(created_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"')
                                  AS created_at_iso,
               to_char(last_delivered_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"')
                                  AS last_delivered_at_iso",
            &[&org_id, &url, &secret, &event_filter, &created_by],
        )
        .await
        .map_err(|e| WebhookError::Sql(format!("insert webhook_subscription: {e}")))?;
    let sub = WebhookSubscription {
        id: row.get("id_text"),
        org_id: row.get("org_id_text"),
        url: row.get("url"),
        event_filter: row.get("event_filter"),
        created_by: row.get("created_by_text"),
        created_at: row.get("created_at_iso"),
        last_delivered_at: row.try_get("last_delivered_at_iso").ok(),
    };
    Ok(CreatedSubscription {
        subscription: sub,
        secret,
    })
}

/// List subscriptions for an org, newest first.
///
/// # Errors
///
/// [`WebhookError::GetConn`] / [`WebhookError::Sql`] on DB issues.
pub async fn list_subscriptions(
    pool: &Pool,
    org_id: &str,
) -> Result<Vec<WebhookSubscription>, WebhookError> {
    let client = pool
        .get()
        .await
        .map_err(|e| WebhookError::GetConn(format!("list_subscriptions: {e}")))?;
    let rows = client
        .query(
            "SELECT
               id::text          AS id_text,
               org_id::text      AS org_id_text,
               url,
               event_filter,
               created_by::text  AS created_by_text,
               to_char(created_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"')
                                  AS created_at_iso,
               to_char(last_delivered_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"')
                                  AS last_delivered_at_iso
             FROM webhook_subscriptions
             WHERE org_id = $1::uuid
             ORDER BY created_at DESC",
            &[&org_id],
        )
        .await
        .map_err(|e| WebhookError::Sql(format!("list webhook_subscriptions: {e}")))?;
    Ok(rows
        .into_iter()
        .map(|r| WebhookSubscription {
            id: r.get("id_text"),
            org_id: r.get("org_id_text"),
            url: r.get("url"),
            event_filter: r.get("event_filter"),
            created_by: r.get("created_by_text"),
            created_at: r.get("created_at_iso"),
            last_delivered_at: r.try_get("last_delivered_at_iso").ok(),
        })
        .collect())
}

/// Delete a subscription scoped to an org. Idempotent — returns
/// `Ok(false)` when no row matched. The `org_id` scope means a
/// caller can only delete subscriptions in their own org.
///
/// # Errors
///
/// [`WebhookError::GetConn`] / [`WebhookError::Sql`] on DB issues.
pub async fn delete_subscription(
    pool: &Pool,
    org_id: &str,
    subscription_id: &str,
) -> Result<bool, WebhookError> {
    let client = pool
        .get()
        .await
        .map_err(|e| WebhookError::GetConn(format!("delete_subscription: {e}")))?;
    let rows = client
        .execute(
            "DELETE FROM webhook_subscriptions
             WHERE id = $1::uuid AND org_id = $2::uuid",
            &[&subscription_id, &org_id],
        )
        .await
        .map_err(|e| WebhookError::Sql(format!("delete webhook_subscription: {e}")))?;
    Ok(rows > 0)
}

/// Resolve a subscription's secret for outbound signing. Used by the
/// dispatcher when a subscription matches an event; never exposed to
/// the HTTP surface.
///
/// # Errors
///
/// [`WebhookError::GetConn`] / [`WebhookError::Sql`] on DB issues.
pub async fn subscription_secret(
    pool: &Pool,
    subscription_id: &str,
) -> Result<Option<(String, String)>, WebhookError> {
    let client = pool
        .get()
        .await
        .map_err(|e| WebhookError::GetConn(format!("subscription_secret: {e}")))?;
    let row = client
        .query_opt(
            "SELECT url, secret FROM webhook_subscriptions WHERE id = $1::uuid",
            &[&subscription_id],
        )
        .await
        .map_err(|e| WebhookError::Sql(format!("lookup webhook_subscription: {e}")))?;
    Ok(row.map(|r| (r.get("url"), r.get("secret"))))
}

/// One subscription with its delivery secret — only the fan-out path
/// needs the secret. Distinct from [`WebhookSubscription`] (which
/// suppresses the secret for the list endpoint).
#[derive(Debug, Clone)]
pub struct SubscriptionWithSecret {
    pub id: String,
    pub url: String,
    pub secret: String,
    pub event_filter: String,
}

/// List active subscriptions for an org, including each row's signing
/// secret. Internal — never returned to the HTTP surface; only the
/// audit fan-out path consumes this.
///
/// # Errors
///
/// [`WebhookError::GetConn`] / [`WebhookError::Sql`] on DB issues.
pub async fn list_for_fanout(
    pool: &Pool,
    org_id: &str,
) -> Result<Vec<SubscriptionWithSecret>, WebhookError> {
    let client = pool
        .get()
        .await
        .map_err(|e| WebhookError::GetConn(format!("list_for_fanout: {e}")))?;
    let rows = client
        .query(
            "SELECT id::text AS id_text, url, secret, event_filter
             FROM webhook_subscriptions
             WHERE org_id = $1::uuid",
            &[&org_id],
        )
        .await
        .map_err(|e| WebhookError::Sql(format!("list_for_fanout query: {e}")))?;
    Ok(rows
        .into_iter()
        .map(|r| SubscriptionWithSecret {
            id: r.get("id_text"),
            url: r.get("url"),
            secret: r.get("secret"),
            event_filter: r.get("event_filter"),
        })
        .collect())
}

/// Mark a successful delivery — touch `last_delivered_at`.
///
/// # Errors
///
/// [`WebhookError::GetConn`] / [`WebhookError::Sql`] on DB issues.
pub async fn mark_delivered(pool: &Pool, subscription_id: &str) -> Result<(), WebhookError> {
    let client = pool
        .get()
        .await
        .map_err(|e| WebhookError::GetConn(format!("mark_delivered: {e}")))?;
    client
        .execute(
            "UPDATE webhook_subscriptions SET last_delivered_at = now() WHERE id = $1::uuid",
            &[&subscription_id],
        )
        .await
        .map_err(|e| WebhookError::Sql(format!("touch last_delivered_at: {e}")))?;
    Ok(())
}

// ── Dispatcher ─────────────────────────────────────────────────────────────

/// Outcome of one [`WebhookDispatcher::deliver`] call.
#[derive(Debug, Clone)]
pub struct DeliveryOutcome {
    /// Final HTTP status reached. `None` when every attempt failed
    /// before getting a response (network error, timeout).
    pub final_status: Option<u16>,
    /// Number of attempts made. 1 ≤ attempts ≤ [`RETRY_BACKOFF.len`].
    pub attempts: usize,
    /// `true` when the final attempt returned a 2xx.
    pub succeeded: bool,
}

/// Outbound webhook dispatcher. One instance per cloud deployment.
///
/// Holds a long-lived `reqwest::Client` so multiple deliveries share
/// the same TLS pool. The signing happens fresh per delivery — the
/// secret never leaves the per-call scope.
#[derive(Debug, Clone)]
pub struct WebhookDispatcher {
    http: reqwest::Client,
}

impl WebhookDispatcher {
    /// Build with the workspace's standard reqwest client (TLS via
    /// rustls + the Mozilla CA bundle, matching the rest of cloud).
    ///
    /// # Errors
    ///
    /// Returns a stringified reqwest error if the client cannot be
    /// constructed (extremely rare — only on TLS init failure).
    pub fn new() -> Result<Self, String> {
        let http = reqwest::Client::builder()
            .timeout(ATTEMPT_TIMEOUT)
            .build()
            .map_err(|e| format!("build reqwest client: {e}"))?;
        Ok(Self { http })
    }

    /// POST the payload to `url`, signing with `secret`. Up to
    /// [`RETRY_BACKOFF.len`] attempts; success on first 2xx.
    ///
    /// Caller passes the payload as already-serialised JSON bytes so
    /// the HMAC is over the exact bytes the receiver re-hashes.
    /// Re-serialising server-side would risk a whitespace mismatch
    /// (object-key ordering, indentation) that breaks verification.
    pub async fn deliver(&self, url: &str, secret: &str, payload: &[u8]) -> DeliveryOutcome {
        let mut outcome = DeliveryOutcome {
            final_status: None,
            attempts: 0,
            succeeded: false,
        };
        for (attempt_idx, backoff) in RETRY_BACKOFF.iter().enumerate() {
            if attempt_idx > 0 {
                tokio::time::sleep(*backoff).await;
            }
            outcome.attempts = attempt_idx + 1;
            let now_ts = now_unix();
            let signature = sign_payload(secret, now_ts, payload);
            let resp = self
                .http
                .post(url)
                .header("Content-Type", "application/json")
                .header("X-Ministr-Timestamp", now_ts.to_string())
                .header("X-Ministr-Signature", format!("sha256={signature}"))
                .body(payload.to_vec())
                .send()
                .await;
            match resp {
                Ok(r) => {
                    let status = r.status().as_u16();
                    outcome.final_status = Some(status);
                    if r.status().is_success() {
                        outcome.succeeded = true;
                        debug!(url, attempts = outcome.attempts, status, "webhook delivered");
                        return outcome;
                    }
                    warn!(url, status, attempt = outcome.attempts, "webhook non-2xx; retrying");
                }
                Err(e) => {
                    warn!(url, error = %e, attempt = outcome.attempts, "webhook transport error; retrying");
                }
            }
        }
        outcome
    }
}

/// Compute the outbound HMAC-SHA256 signature over
/// `timestamp + "." + body`. The receiver verifies by recomputing the
/// same construction. Mirrors the F1.5 Stripe-inbound verifier in
/// [`crate::billing::stripe`] — the same shape on both sides keeps
/// the wire format self-documenting.
///
/// # Panics
///
/// Practically never — `HMAC-SHA256` accepts any key length per
/// RFC 2104, so the inner `new_from_slice` cannot return `Err` for a
/// `&str` secret. The `expect` is structural rather than fallible.
#[must_use]
pub fn sign_payload(secret: &str, timestamp: u64, payload: &[u8]) -> String {
    let mut mac = <Hmac<Sha256> as Mac>::new_from_slice(secret.as_bytes())
        .expect("HMAC-SHA256 accepts any key length");
    mac.update(timestamp.to_string().as_bytes());
    mac.update(b".");
    mac.update(payload);
    let digest = mac.finalize().into_bytes();
    let mut out = String::with_capacity(digest.len() * 2);
    for b in digest {
        let _ = write!(out, "{b:02x}");
    }
    out
}

/// Does `filter` admit `action`?
///
/// Two cases:
/// - `"*"` → admit every action.
/// - Comma-separated list of action names → admit when at least one
///   token matches `action` exactly (whitespace around tokens is
///   trimmed). Empty tokens are ignored.
///
/// Wildcards / prefix patterns (e.g. `"corpus.*"`) are intentionally
/// NOT supported in v0 — the explicit allowlist is more predictable
/// for admins and matches the closed-vocabulary shape of
/// [`crate::audit::AuditEntry::action`].
#[must_use]
pub fn event_matches_filter(filter: &str, action: &str) -> bool {
    let trimmed = filter.trim();
    if trimmed == "*" {
        return true;
    }
    trimmed
        .split(',')
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .any(|t| t == action)
}

// ── Audit fan-out ──────────────────────────────────────────────────────────

/// JSON payload posted to webhook receivers. Stable wire shape;
/// customers who code against this should pin to the `schema`
/// version field for forward-compat.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct WebhookPayload<'a> {
    /// Schema version — bump when the payload shape changes
    /// incompatibly. v0 keeps a flat `action` + `resource` + `org_id`
    /// + `actor` + `ts` shape.
    schema: u32,
    /// The audit action name (`api_key.created`, `share.granted`, …).
    event: &'a str,
    /// Stable identifier of the affected resource. Shape depends on
    /// the action — e.g. `share.*` rows encode `corpus_id:org_id`.
    resource: &'a str,
    /// Org context, always present at the fan-out layer.
    org_id: &'a str,
    /// Subject of the user / api-key that triggered the action.
    actor: Option<&'a str>,
    /// Server timestamp (unix seconds) — the same value the HMAC
    /// header carries, so receivers don't need a clock dependency
    /// to render the event.
    ts: u64,
}

/// [`AuditSink`] impl that fans audit events out to matching
/// [`webhook_subscriptions`]. Self-hosted serve never wires this;
/// cloud builds one alongside the [`crate::audit::PostgresAuditSink`]
/// and composes both via [`ChainedAuditSink`].
///
/// Per call: spawns a tokio task that lists the org's subscriptions,
/// filters by `event_filter`, and dispatches each match in parallel.
/// The dispatcher's own per-attempt timeout + retry shape applies —
/// see [`WebhookDispatcher`].
#[derive(Debug, Clone)]
pub struct WebhookFanoutSink {
    pool: Arc<Pool>,
    dispatcher: Arc<WebhookDispatcher>,
}

impl WebhookFanoutSink {
    /// Bind a fan-out to a pool + dispatcher. The dispatcher is
    /// typically the same `Arc<WebhookDispatcher>` mounted on
    /// [`WebhooksState`] so multiple call sites share TLS connections.
    #[must_use]
    pub fn new(pool: Arc<Pool>, dispatcher: Arc<WebhookDispatcher>) -> Self {
        Self { pool, dispatcher }
    }

    /// Wrap as `Arc<dyn AuditSink>` for [`ChainedAuditSink`].
    #[must_use]
    pub fn into_dyn(self) -> Arc<dyn AuditSink> {
        Arc::new(self)
    }
}

impl AuditSink for WebhookFanoutSink {
    fn record(&self, entry: AuditEntry) {
        // User-level events (org.created, api_key.created on a
        // personal account) have no org context — no subscriptions
        // exist to match. Skip silently.
        let Some(org_id) = entry.org_id.clone() else {
            return;
        };
        let pool = Arc::clone(&self.pool);
        let dispatcher = Arc::clone(&self.dispatcher);
        tokio::spawn(async move {
            let subs = match list_for_fanout(&pool, &org_id).await {
                Ok(s) => s,
                Err(e) => {
                    warn!(error = %e, org_id, "webhook fan-out: list_for_fanout failed");
                    return;
                }
            };
            for sub in subs {
                if !event_matches_filter(&sub.event_filter, &entry.action) {
                    continue;
                }
                let ts = now_unix();
                let payload = WebhookPayload {
                    schema: 1,
                    event: &entry.action,
                    resource: &entry.resource,
                    org_id: &org_id,
                    actor: entry.actor.as_deref(),
                    ts,
                };
                let bytes = match serde_json::to_vec(&payload) {
                    Ok(b) => b,
                    Err(e) => {
                        warn!(
                            error = %e,
                            subscription_id = %sub.id,
                            "webhook fan-out: serialise payload failed"
                        );
                        continue;
                    }
                };
                let outcome = dispatcher.deliver(&sub.url, &sub.secret, &bytes).await;
                if outcome.succeeded {
                    if let Err(e) = mark_delivered(&pool, &sub.id).await {
                        warn!(
                            error = %e,
                            subscription_id = %sub.id,
                            "webhook fan-out: mark_delivered failed"
                        );
                    }
                    debug!(
                        subscription_id = %sub.id,
                        action = %entry.action,
                        attempts = outcome.attempts,
                        "webhook delivered"
                    );
                } else {
                    warn!(
                        subscription_id = %sub.id,
                        action = %entry.action,
                        attempts = outcome.attempts,
                        final_status = ?outcome.final_status,
                        "webhook delivery FAILED after retries"
                    );
                }
            }
        });
    }
}

/// Compose multiple [`AuditSink`]s — calls `record` on each. Used by
/// `cmd_serve_http` to chain [`crate::audit::PostgresAuditSink`]
/// (persistent log) with [`WebhookFanoutSink`] (outbound dispatch).
///
/// Both sinks are fire-and-forget so the chain itself never blocks.
/// Ordering matters slightly: the Postgres sink should come first
/// because operators expect "audit row written" to be authoritative
/// even if the webhook dispatch is in-flight when they query.
#[derive(Clone)]
pub struct ChainedAuditSink {
    sinks: Vec<Arc<dyn AuditSink>>,
}

impl std::fmt::Debug for ChainedAuditSink {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ChainedAuditSink")
            .field("sink_count", &self.sinks.len())
            .finish()
    }
}

impl ChainedAuditSink {
    /// Build from an ordered list of sinks.
    #[must_use]
    pub fn new(sinks: Vec<Arc<dyn AuditSink>>) -> Self {
        Self { sinks }
    }
}

impl AuditSink for ChainedAuditSink {
    fn record(&self, entry: AuditEntry) {
        for sink in &self.sinks {
            sink.record(entry.clone());
        }
    }
}

/// Mint a fresh 32-byte secret as base64url-no-pad.
fn mint_secret() -> String {
    let mut buf = [0u8; SECRET_BYTES];
    getrandom_fill(&mut buf).expect("OS RNG must be available for webhook secrets");
    base64_url_no_pad(&buf)
}

fn base64_url_no_pad(data: &[u8]) -> String {
    const ALPHABET: &[u8; 64] =
        b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    let mut i = 0;
    while i < data.len() {
        let b0 = data[i];
        let b1 = data.get(i + 1).copied().unwrap_or(0);
        let b2 = data.get(i + 2).copied().unwrap_or(0);
        let triplet = (u32::from(b0) << 16) | (u32::from(b1) << 8) | u32::from(b2);
        out.push(ALPHABET[((triplet >> 18) & 0x3f) as usize] as char);
        out.push(ALPHABET[((triplet >> 12) & 0x3f) as usize] as char);
        if i + 1 < data.len() {
            out.push(ALPHABET[((triplet >> 6) & 0x3f) as usize] as char);
        }
        if i + 2 < data.len() {
            out.push(ALPHABET[(triplet & 0x3f) as usize] as char);
        }
        i += 3;
    }
    out
}

fn now_unix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

// ── Routes ─────────────────────────────────────────────────────────────────

/// Axum state for the webhooks router.
#[derive(Clone)]
pub struct WebhooksState {
    pub pool: Arc<Pool>,
    pub dispatcher: Arc<WebhookDispatcher>,
}

impl std::fmt::Debug for WebhooksState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WebhooksState").finish_non_exhaustive()
    }
}

impl WebhooksState {
    #[must_use]
    pub fn new(pool: Arc<Pool>, dispatcher: Arc<WebhookDispatcher>) -> Self {
        Self { pool, dispatcher }
    }
}

/// Build the webhooks router.
pub fn webhooks_routes(state: WebhooksState) -> Router {
    Router::new()
        .route(
            "/api/v1/orgs/{id}/webhooks",
            post(create_handler).get(list_handler),
        )
        .route(
            "/api/v1/orgs/{id}/webhooks/{wid}",
            delete(delete_handler),
        )
        .route(
            "/api/v1/orgs/{id}/webhooks/{wid}/test",
            post(test_handler),
        )
        .with_state(state)
}

#[derive(Debug, Deserialize)]
struct CreateRequest {
    url: String,
    #[serde(default)]
    event_filter: Option<String>,
}

#[derive(Debug, Serialize)]
struct ListResponse {
    subscriptions: Vec<WebhookSubscription>,
}

#[derive(Debug, Serialize)]
struct TestResponse {
    final_status: Option<u16>,
    attempts: usize,
    succeeded: bool,
}

#[derive(Debug)]
enum WebhooksApiError {
    Unauthenticated,
    Forbidden,
    NotFound,
    Invalid(&'static str),
    Repo(WebhookError),
}

impl axum::response::IntoResponse for WebhooksApiError {
    fn into_response(self) -> axum::response::Response {
        use axum::http::StatusCode as S;
        match self {
            Self::Unauthenticated => (S::UNAUTHORIZED, "unauthenticated").into_response(),
            Self::Forbidden => (S::FORBIDDEN, "forbidden").into_response(),
            Self::NotFound => (S::NOT_FOUND, "not_found").into_response(),
            Self::Invalid(msg) => (S::BAD_REQUEST, msg).into_response(),
            Self::Repo(e) => {
                warn!(error = %e, "webhooks repo error");
                (S::INTERNAL_SERVER_ERROR, "internal").into_response()
            }
        }
    }
}

/// Common authz path: caller must be owner/admin of the target org.
/// Mirrors `audit::list_handler` — members can't manage webhooks.
async fn assert_owner_or_admin(
    pool: &Pool,
    org_id: &str,
    user_id: &str,
) -> Result<(), WebhooksApiError> {
    let role = member_role(pool, org_id, user_id)
        .await
        .map_err(|e| WebhooksApiError::Repo(WebhookError::Sql(e.to_string())))?;
    if !matches!(role.as_deref(), Some("owner" | "admin")) {
        return Err(WebhooksApiError::Forbidden);
    }
    Ok(())
}

async fn create_handler(
    State(state): State<WebhooksState>,
    tenant: Option<Extension<Tenant>>,
    Path(org_id): Path<String>,
    Json(body): Json<CreateRequest>,
) -> Result<(StatusCode, Json<CreatedSubscription>), WebhooksApiError> {
    let tenant = tenant.ok_or(WebhooksApiError::Unauthenticated)?;
    assert_owner_or_admin(&state.pool, &org_id, &tenant.0.subject).await?;
    let url = body.url.trim();
    if url.is_empty() || !(url.starts_with("https://") || url.starts_with("http://")) {
        return Err(WebhooksApiError::Invalid(
            "url must be a non-empty http(s) URL",
        ));
    }
    let filter = body
        .event_filter
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("*")
        .to_owned();
    let created = create_subscription(&state.pool, &org_id, url, &filter, &tenant.0.subject)
        .await
        .map_err(WebhooksApiError::Repo)?;
    Ok((StatusCode::CREATED, Json(created)))
}

async fn list_handler(
    State(state): State<WebhooksState>,
    tenant: Option<Extension<Tenant>>,
    Path(org_id): Path<String>,
) -> Result<Json<ListResponse>, WebhooksApiError> {
    let tenant = tenant.ok_or(WebhooksApiError::Unauthenticated)?;
    assert_owner_or_admin(&state.pool, &org_id, &tenant.0.subject).await?;
    let subs = list_subscriptions(&state.pool, &org_id)
        .await
        .map_err(WebhooksApiError::Repo)?;
    Ok(Json(ListResponse {
        subscriptions: subs,
    }))
}

async fn delete_handler(
    State(state): State<WebhooksState>,
    tenant: Option<Extension<Tenant>>,
    Path((org_id, subscription_id)): Path<(String, String)>,
) -> Result<StatusCode, WebhooksApiError> {
    let tenant = tenant.ok_or(WebhooksApiError::Unauthenticated)?;
    assert_owner_or_admin(&state.pool, &org_id, &tenant.0.subject).await?;
    let removed = delete_subscription(&state.pool, &org_id, &subscription_id)
        .await
        .map_err(WebhooksApiError::Repo)?;
    if removed {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(WebhooksApiError::NotFound)
    }
}

async fn test_handler(
    State(state): State<WebhooksState>,
    tenant: Option<Extension<Tenant>>,
    Path((org_id, subscription_id)): Path<(String, String)>,
) -> Result<Json<TestResponse>, WebhooksApiError> {
    let tenant = tenant.ok_or(WebhooksApiError::Unauthenticated)?;
    assert_owner_or_admin(&state.pool, &org_id, &tenant.0.subject).await?;
    let Some((url, secret)) = subscription_secret(&state.pool, &subscription_id)
        .await
        .map_err(WebhooksApiError::Repo)?
    else {
        return Err(WebhooksApiError::NotFound);
    };
    // Synthetic payload — distinct shape so receivers can branch on
    // `event` to avoid mistakenly processing a test as a real event.
    let payload = serde_json::json!({
        "event": "ministr.test",
        "subscription_id": subscription_id,
        "org_id": org_id,
        "ts": now_unix(),
    });
    let bytes =
        serde_json::to_vec(&payload).map_err(|e| WebhooksApiError::Repo(WebhookError::Sql(e.to_string())))?;
    let outcome = state.dispatcher.deliver(&url, &secret, &bytes).await;
    if outcome.succeeded
        && let Err(e) = mark_delivered(&state.pool, &subscription_id).await
    {
        warn!(error = %e, subscription_id, "mark_delivered failed after test");
    }
    Ok(Json(TestResponse {
        final_status: outcome.final_status,
        attempts: outcome.attempts,
        succeeded: outcome.succeeded,
    }))
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sign_payload_is_deterministic_for_same_inputs() {
        let a = sign_payload("secret", 1_715_000_000, b"{\"hello\":\"world\"}");
        let b = sign_payload("secret", 1_715_000_000, b"{\"hello\":\"world\"}");
        assert_eq!(a, b);
        assert_eq!(a.len(), 64);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn sign_payload_differs_when_timestamp_changes() {
        let a = sign_payload("secret", 1, b"body");
        let b = sign_payload("secret", 2, b"body");
        assert_ne!(a, b);
    }

    #[test]
    fn sign_payload_differs_when_secret_changes() {
        let a = sign_payload("secret-a", 1, b"body");
        let b = sign_payload("secret-b", 1, b"body");
        assert_ne!(a, b);
    }

    #[test]
    fn sign_payload_matches_stripe_inbound_construction() {
        // Cross-check: the F1.5 Stripe webhook verifier in
        // crate::billing::stripe constructs HMAC over
        // `timestamp.to_string() + "." + body`. Our outbound signer
        // must use the same shape so a customer who already has a
        // Stripe-receiver lying around can point it at us with only
        // a header-name change.
        let ts = 1_715_000_000_u64;
        let body = b"{\"foo\":1}";
        let our_sig = sign_payload("k", ts, body);

        // Re-implement the inbound construction inline (sanity check).
        let mut mac = <Hmac<Sha256> as Mac>::new_from_slice(b"k").unwrap();
        mac.update(ts.to_string().as_bytes());
        mac.update(b".");
        mac.update(body);
        let expected_bytes = mac.finalize().into_bytes();
        let mut expected = String::with_capacity(64);
        for b in expected_bytes {
            let _ = write!(expected, "{b:02x}");
        }
        assert_eq!(our_sig, expected);
    }

    #[test]
    fn minted_secret_is_url_safe_and_correct_length() {
        let s = mint_secret();
        // 32 bytes base64url-no-pad = 43 chars
        assert!(s.len() >= 40 && s.len() <= 44);
        assert!(s
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_'));
    }

    #[test]
    fn event_filter_star_admits_everything() {
        assert!(event_matches_filter("*", "anything"));
        assert!(event_matches_filter("*", "share.granted"));
        assert!(event_matches_filter(" * ", "with.whitespace"));
    }

    #[test]
    fn event_filter_csv_matches_exact_only() {
        // Exact match in a CSV list — both single-token and multi-token.
        assert!(event_matches_filter("share.granted", "share.granted"));
        assert!(event_matches_filter(
            "share.granted, api_key.created",
            "api_key.created"
        ));
        // Non-match.
        assert!(!event_matches_filter("share.granted", "share.revoked"));
        // Prefix patterns are NOT admitted in v0.
        assert!(!event_matches_filter("corpus.*", "corpus.created"));
        // Empty filter and empty tokens.
        assert!(!event_matches_filter("", "anything"));
        assert!(!event_matches_filter(", ,", "anything"));
    }

    #[test]
    fn event_filter_trims_whitespace_around_tokens() {
        assert!(event_matches_filter(
            "  share.granted ,  api_key.created  ",
            "share.granted"
        ));
        assert!(event_matches_filter(
            "  share.granted ,  api_key.created  ",
            "api_key.created"
        ));
    }

    #[derive(Debug, Default)]
    struct CountingSink {
        count: std::sync::Mutex<u32>,
    }
    impl AuditSink for CountingSink {
        fn record(&self, _entry: AuditEntry) {
            *self.count.lock().unwrap() += 1;
        }
    }

    #[test]
    fn chained_audit_sink_fans_to_every_sink() {
        let a = Arc::new(CountingSink::default());
        let b = Arc::new(CountingSink::default());
        let chain = ChainedAuditSink::new(vec![
            Arc::clone(&a) as Arc<dyn AuditSink>,
            Arc::clone(&b) as Arc<dyn AuditSink>,
        ]);
        chain.record(AuditEntry::new("share.granted", "corpus:org"));
        chain.record(AuditEntry::new("api_key.created", "key"));
        assert_eq!(*a.count.lock().unwrap(), 2);
        assert_eq!(*b.count.lock().unwrap(), 2);
    }

    #[test]
    fn retry_backoff_is_short_then_longer() {
        // Sanity check the shape: monotonic non-decreasing, first
        // attempt is immediate. If someone reorders these to "30, 5, 0"
        // the delivery wall-clock characteristics change drastically.
        assert_eq!(RETRY_BACKOFF.len(), 3);
        assert_eq!(RETRY_BACKOFF[0], Duration::ZERO);
        assert!(RETRY_BACKOFF[1] < RETRY_BACKOFF[2]);
    }
}
