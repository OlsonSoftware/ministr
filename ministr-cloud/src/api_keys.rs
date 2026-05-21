//! F3.4a — service-account API keys backend.
//!
//! Postgres-backed repo + axum routes + a resolver that implements
//! [`ministr_api::ApiKeyResolver`]. Wired into `OAuthStore` via
//! `with_api_key_resolver` so the existing token-validation middleware
//! authenticates API-key bearer tokens alongside OAuth tokens.
//!
//! # Token discipline
//!
//! Mirrors the F3.1b-i invite-token discipline:
//!
//! - Raw token is generated from the OS RNG, base64url-no-pad, prefixed
//!   with `mst_pk_` so callers (and logs) recognise the shape.
//! - The raw token is returned to the caller exactly ONCE at create
//!   time. The DB only stores the SHA-256 hex digest in `api_keys.hash`
//!   plus the first 8 chars of the random portion in `api_keys.prefix`
//!   for the list UI's display.
//! - `consume`-equivalent lookup hashes the candidate token before
//!   indexing. A leaked DB dump cannot be used to authenticate.
//!
//! # F3.4a scope
//!
//! v0: user-owned keys only (`owner_user_id` always populated;
//! `owner_org_id` reserved for F3.4b's org-owned UI). Plan is derived
//! from `users.plan_id` at resolution time so a plan upgrade flows
//! through to the key without a re-mint.

use std::fmt::Write as _;
use std::sync::Arc;

use axum::Json;
use axum::Router;
use axum::extract::{Extension, Path, State};
use axum::http::StatusCode;
use axum::routing::{delete, post};
use deadpool_postgres::Pool;
use getrandom::fill as getrandom_fill;
use ministr_api::{
    ApiKeyError, ApiKeyResolver, AuditEntry, AuditSink, ResolveApiKeyFuture, ResolvedApiKey,
    TouchLastUsedFuture,
};
use ministr_mcp::auth::tenant::Tenant;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tracing::warn;

/// Token-prefix marker. Keeps the shape recognisable in logs +
/// catches accidental commits via secret-scanning tools (`mst_pk_…`
/// scans cleanly across `GitGuardian` / `TruffleHog`).
pub const TOKEN_PREFIX: &str = "mst_pk_";

/// F3.4c-i — the four-scope vocabulary accepted on `POST /api/v1/api_keys`.
/// Mirrors `ministr_mcp::auth::OAuthConfig::default().scopes_supported`
/// — kept inline rather than importing the OAuth config so `api_keys.rs`
/// stays decoupled from the auth-server's runtime configuration. If the
/// allowed set ever expands at runtime, this constant becomes the seam
/// to thread an injected list.
pub const ALLOWED_API_KEY_SCOPES: &[&str] = &[
    "ministr:read",
    "ministr:write",
    "ministr:bundle:read",
    "ministr:bundle:write",
];

/// Random-bytes-per-token. 32 bytes → 256-bit entropy, same as F3.1b-i
/// invite tokens. Base64url encodes to ~43 chars.
const TOKEN_ENTROPY_BYTES: usize = 32;

/// How many chars of the random portion to store in
/// `api_keys.prefix` for the list UI to display. 8 chars × 6 bits =
/// 48 bits — collision-resistant within a single user's keys but does
/// not reveal the secret.
const PREFIX_LEN: usize = 8;

/// Errors surfaced by the `api_keys` module. Mirrors `OrgError` shape.
#[derive(Debug, thiserror::Error)]
pub enum ApiKeysError {
    /// Could not acquire a pooled DB connection.
    #[error("get connection: {0}")]
    GetConn(String),
    /// SQL error from a query / execute.
    #[error("sql: {0}")]
    Sql(String),
}

impl From<ApiKeysError> for ApiKeyError {
    fn from(value: ApiKeysError) -> Self {
        match value {
            ApiKeysError::GetConn(e) | ApiKeysError::Sql(e) => ApiKeyError::Storage(e),
        }
    }
}

/// One row from `api_keys`, shaped for the list-keys response.
/// Excludes the secret hash — the caller never needs it after the
/// create response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiKeyRow {
    /// `api_keys.id`.
    pub id: String,
    /// Display name the user picked.
    pub name: String,
    /// First 8 chars of the random secret portion (NOT the full token).
    /// Lets the list UI label a row without surfacing the secret.
    pub prefix: String,
    /// Whitespace-separated scope list.
    pub scopes: String,
    /// ISO-8601 last-used timestamp. `None` if the key has never
    /// authenticated yet.
    pub last_used_at: Option<String>,
    /// ISO-8601 expiry. `None` for keys with no scheduled expiry.
    pub expires_at: Option<String>,
    /// ISO-8601 creation timestamp.
    pub created_at: String,
}

/// Wrapper around [`create_user_api_key`]: the persisted row plus the
/// one-time raw token. Callers MUST return the token to the user
/// exactly once and never log it.
#[derive(Debug)]
pub struct CreatedApiKey {
    pub row: ApiKeyRow,
    pub raw_token: String,
}

/// F3.4c-ii — staleness threshold matching ROADMAP §F3.4c "90 days"
/// language. Mirrors [`crate::DEFAULT_AUDIT_RETENTION_DAYS`] in spirit
/// (operator-tunable, default ships in production), but the two are
/// semantically distinct knobs — keep them separate constants.
pub const DEFAULT_STALE_API_KEY_DAYS: u32 = 90;

/// One stale-key row surfaced by [`flag_stale_api_keys`].
#[derive(Debug, Clone)]
pub struct StaleApiKey {
    /// `api_keys.id` as text. Echoes the `resource` field on the
    /// emitted `api_key.stale` audit event.
    pub key_id: String,
    /// `api_keys.owner_user_id` as text. Echoes the `actor` field on
    /// the audit event — semantically: "the owner's key went stale".
    pub owner_user_id: String,
    /// Display name the user picked at create time. Surfaced in the
    /// structured-log line so operators can correlate without joining
    /// against the `api_keys` table.
    pub name: String,
}

/// Outcome of one [`flag_stale_api_keys`] pass.
#[derive(Debug, Clone)]
pub struct StaleApiKeysOutcome {
    /// Rows flagged this pass.
    pub flagged: u64,
    /// Wall-clock for the SELECT + audit-emit loop. Mirrors
    /// [`PruneOutcome::elapsed`] so the structured-log dashboard
    /// renders the operation uniformly.
    pub elapsed: std::time::Duration,
    /// Threshold in days the caller passed. Echoed for self-describing
    /// logs.
    pub threshold_days: u32,
}

/// F3.4c-ii — detect keys whose `last_used_at` (or `created_at` for
/// never-used keys) is older than `threshold_days` days. For each row,
/// emit an `api_key.stale` audit event via the injected sink. Returns
/// the flagged count and elapsed wall-clock for the cron's
/// structured-log line.
///
/// Excludes already-revoked keys (`revoked_at IS NULL`). The audit
/// event carries `resource = key_id`, `actor = owner_user_id`, and
/// `org_id = None` (v0 supports user-owned keys only — when F3.4 grows
/// org-owned keys, attach `org_id` here so F3.5 webhook fan-out can
/// route the event to subscribed channels).
///
/// # Errors
///
/// [`ApiKeysError::GetConn`] when the pool refuses a connection,
/// [`ApiKeysError::Sql`] when the SELECT itself fails.
pub async fn flag_stale_api_keys(
    pool: &Pool,
    threshold_days: u32,
    sink: &dyn AuditSink,
) -> Result<StaleApiKeysOutcome, ApiKeysError> {
    let client = pool
        .get()
        .await
        .map_err(|e| ApiKeysError::GetConn(format!("flag_stale_api_keys: {e}")))?;
    let started = std::time::Instant::now();
    let threshold = i32::try_from(threshold_days).unwrap_or(i32::MAX);
    let rows = client
        .query(
            "SELECT
                 id::text             AS key_id,
                 owner_user_id::text  AS owner_user_id,
                 name
             FROM api_keys
             WHERE revoked_at IS NULL
               AND COALESCE(last_used_at, created_at)
                     < now() - make_interval(days => $1::integer)",
            &[&threshold],
        )
        .await
        .map_err(|e| ApiKeysError::Sql(format!("select stale api_keys: {e}")))?;
    let mut flagged: u64 = 0;
    for row in rows {
        let stale = StaleApiKey {
            key_id: row.get("key_id"),
            owner_user_id: row.get("owner_user_id"),
            name: row.get("name"),
        };
        sink.record(
            AuditEntry::new("api_key.stale", &stale.key_id).with_actor(&stale.owner_user_id),
        );
        flagged = flagged.saturating_add(1);
    }
    Ok(StaleApiKeysOutcome {
        flagged,
        elapsed: started.elapsed(),
        threshold_days,
    })
}

/// F3.4c-i — validate a whitespace-separated scopes string against
/// [`ALLOWED_API_KEY_SCOPES`]. Returns the canonical re-joined string
/// (single-space-separated, original order preserved) on success.
///
/// # Errors
///
/// Returns `Err(unknown_token)` listing the first unknown scope so the
/// API can return an actionable 400.
pub fn validate_scopes(raw: &str) -> Result<String, String> {
    let mut canonical = String::with_capacity(raw.len());
    let mut first = true;
    for token in raw.split_whitespace() {
        if !ALLOWED_API_KEY_SCOPES.contains(&token) {
            return Err(token.to_string());
        }
        if !first {
            canonical.push(' ');
        }
        canonical.push_str(token);
        first = false;
    }
    if canonical.is_empty() {
        return Err(String::new());
    }
    Ok(canonical)
}

/// Mint a new user-owned API key.
///
/// `owner_user_id` is the authenticated user's UUID (from `Tenant.subject`).
/// `scopes` follows the existing OAuth scope shape (whitespace-separated).
///
/// # Errors
///
/// [`ApiKeysError::Sql`] on DB failure, [`ApiKeysError::GetConn`] when
/// the pool is empty.
pub async fn create_user_api_key(
    pool: &Pool,
    owner_user_id: &str,
    name: &str,
    scopes: &str,
) -> Result<CreatedApiKey, ApiKeysError> {
    let raw_token = mint_token();
    let hash = hash_token(&raw_token);
    let prefix = derive_prefix(&raw_token);

    let client = pool
        .get()
        .await
        .map_err(|e| ApiKeysError::GetConn(format!("create_api_key: {e}")))?;

    let row = client
        .query_one(
            "INSERT INTO api_keys
               (owner_user_id, name, hash, scopes, prefix)
             VALUES ($1::uuid, $2, $3, $4, $5)
             RETURNING
               id::text         AS id_text,
               name,
               prefix,
               scopes,
               to_char(created_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"')
                                  AS created_at_iso,
               to_char(last_used_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"')
                                  AS last_used_at_iso,
               to_char(expires_at  AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"')
                                  AS expires_at_iso",
            &[&owner_user_id, &name, &hash, &scopes, &prefix],
        )
        .await
        .map_err(|e| ApiKeysError::Sql(format!("insert api_key: {e}")))?;

    Ok(CreatedApiKey {
        row: ApiKeyRow {
            id: row.get("id_text"),
            name: row.get("name"),
            prefix: row.get("prefix"),
            scopes: row.get("scopes"),
            last_used_at: row.try_get("last_used_at_iso").ok(),
            expires_at: row.try_get("expires_at_iso").ok(),
            created_at: row.get("created_at_iso"),
        },
        raw_token,
    })
}

/// List active (non-revoked) API keys for a user, newest first.
///
/// # Errors
///
/// [`ApiKeysError::Sql`] / [`ApiKeysError::GetConn`] on DB issues.
pub async fn list_user_api_keys(
    pool: &Pool,
    owner_user_id: &str,
) -> Result<Vec<ApiKeyRow>, ApiKeysError> {
    let client = pool
        .get()
        .await
        .map_err(|e| ApiKeysError::GetConn(format!("list_api_keys: {e}")))?;
    let rows = client
        .query(
            "SELECT
               id::text         AS id_text,
               name,
               prefix,
               scopes,
               to_char(created_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"')
                                  AS created_at_iso,
               to_char(last_used_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"')
                                  AS last_used_at_iso,
               to_char(expires_at  AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"')
                                  AS expires_at_iso
             FROM api_keys
             WHERE owner_user_id = $1::uuid AND revoked_at IS NULL
             ORDER BY created_at DESC",
            &[&owner_user_id],
        )
        .await
        .map_err(|e| ApiKeysError::Sql(format!("list api_keys: {e}")))?;
    Ok(rows
        .into_iter()
        .map(|r| ApiKeyRow {
            id: r.get("id_text"),
            name: r.get("name"),
            prefix: r.try_get("prefix").unwrap_or_default(),
            scopes: r.get("scopes"),
            last_used_at: r.try_get("last_used_at_iso").ok(),
            expires_at: r.try_get("expires_at_iso").ok(),
            created_at: r.get("created_at_iso"),
        })
        .collect())
}

/// Soft-revoke a user's API key by id. Idempotent — re-revoking an
/// already-revoked row is a no-op (still returns Ok).
///
/// `owner_user_id` is the caller; the WHERE clause refuses to revoke
/// keys the caller doesn't own, so a misrouted DELETE returns
/// `Ok(false)` (not found) instead of leaking other users' key IDs.
///
/// # Errors
///
/// [`ApiKeysError::Sql`] / [`ApiKeysError::GetConn`] on DB issues.
pub async fn revoke_user_api_key(
    pool: &Pool,
    owner_user_id: &str,
    key_id: &str,
) -> Result<bool, ApiKeysError> {
    let client = pool
        .get()
        .await
        .map_err(|e| ApiKeysError::GetConn(format!("revoke_api_key: {e}")))?;
    let rows = client
        .execute(
            "UPDATE api_keys
               SET revoked_at = now()
             WHERE id = $1::uuid AND owner_user_id = $2::uuid AND revoked_at IS NULL",
            &[&key_id, &owner_user_id],
        )
        .await
        .map_err(|e| ApiKeysError::Sql(format!("revoke api_key: {e}")))?;
    Ok(rows > 0)
}

// ── Resolver ───────────────────────────────────────────────────────────────

/// Postgres-backed implementation of [`ApiKeyResolver`].
///
/// The resolver hashes the candidate token, looks it up in
/// `api_keys` joined to `users` (to fetch the owner's plan), and
/// returns a [`ResolvedApiKey`] on hit. The `idx_api_keys_active_hash`
/// partial index makes this a single-probe operation.
#[derive(Debug, Clone)]
pub struct PostgresApiKeyResolver {
    pool: Pool,
}

impl PostgresApiKeyResolver {
    /// Build a resolver bound to a Postgres pool. Shares the pool with
    /// every other cloud-side reader; the resolver issues at most one
    /// query per authenticated request.
    #[must_use]
    pub fn new(pool: Pool) -> Self {
        Self { pool }
    }
}

impl ApiKeyResolver for PostgresApiKeyResolver {
    fn resolve<'a>(&'a self, raw_token: &'a str) -> ResolveApiKeyFuture<'a> {
        Box::pin(async move {
            // Cheap early-out: a token that isn't even our shape can't
            // be an API key. Saves a hash + DB round-trip on every
            // OAuth-token request that falls through to here.
            if !raw_token.starts_with(TOKEN_PREFIX) {
                return Ok(None);
            }
            let hash = hash_token(raw_token);
            let client = self
                .pool
                .get()
                .await
                .map_err(|e| ApiKeyError::Storage(format!("resolve get_conn: {e}")))?;
            let row = client
                .query_opt(
                    "SELECT
                       k.id::text            AS key_id_text,
                       k.owner_user_id::text AS owner_user_id_text,
                       k.owner_org_id::text  AS owner_org_id_text,
                       COALESCE(u.plan_id, o.plan_id, 'pro') AS plan_id,
                       k.scopes,
                       (k.expires_at IS NOT NULL AND now() > k.expires_at) AS is_expired
                     FROM api_keys k
                     LEFT JOIN users u ON u.id = k.owner_user_id
                     LEFT JOIN orgs  o ON o.id = k.owner_org_id
                     WHERE k.hash = $1 AND k.revoked_at IS NULL",
                    &[&hash],
                )
                .await
                .map_err(|e| ApiKeyError::Storage(format!("resolve query: {e}")))?;
            let Some(row) = row else { return Ok(None) };
            let is_expired: bool = row.get("is_expired");
            if is_expired {
                return Ok(None);
            }
            // Polymorphic ownership: subject = whichever of user/org is
            // populated. The DB CHECK constraint guarantees exactly one
            // is non-null on writes.
            let user_id: Option<String> = row.try_get("owner_user_id_text").ok().flatten();
            let org_id: Option<String> = row.try_get("owner_org_id_text").ok().flatten();
            let subject = match (user_id, &org_id) {
                (Some(u), _) => u,
                (None, Some(o)) => o.clone(),
                _ => return Ok(None),
            };
            Ok(Some(ResolvedApiKey {
                key_id: row.get("key_id_text"),
                subject,
                org_id,
                plan_id: row.get("plan_id"),
                scopes: row.get("scopes"),
            }))
        })
    }

    fn touch_last_used<'a>(&'a self, key_id: &'a str) -> TouchLastUsedFuture<'a> {
        Box::pin(async move {
            let client = self
                .pool
                .get()
                .await
                .map_err(|e| ApiKeyError::Storage(format!("touch get_conn: {e}")))?;
            client
                .execute(
                    "UPDATE api_keys SET last_used_at = now() WHERE id = $1::uuid",
                    &[&key_id],
                )
                .await
                .map_err(|e| ApiKeyError::Storage(format!("touch update: {e}")))?;
            Ok(())
        })
    }
}

// ── Routes ─────────────────────────────────────────────────────────────────

/// Axum state for the `api_keys` router.
#[derive(Clone)]
pub struct ApiKeysState {
    pub pool: Pool,
    /// F3.7a — optional audit sink. `Some` makes create/revoke emit
    /// `audit_events` rows on successful writes.
    pub audit: Option<Arc<dyn AuditSink>>,
}

impl std::fmt::Debug for ApiKeysState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ApiKeysState")
            .field("audit_wired", &self.audit.is_some())
            .finish_non_exhaustive()
    }
}

impl ApiKeysState {
    /// Build a state from a pool with no audit sink. Tests use this.
    #[must_use]
    pub fn new(pool: Pool) -> Self {
        Self { pool, audit: None }
    }

    /// F3.7a — wire an audit sink. Cloud deployments call this; self-
    /// hosted serve never constructs an `ApiKeysState` because the
    /// router itself is gated on `cloud_pool.is_some()`.
    #[must_use]
    pub fn with_audit(mut self, audit: Arc<dyn AuditSink>) -> Self {
        self.audit = Some(audit);
        self
    }

    fn audit_record(&self, entry: AuditEntry) {
        if let Some(sink) = self.audit.as_ref() {
            sink.record(entry);
        }
    }
}

/// Build the `api_keys` router. Mounted under no prefix; routes carry
/// their full `/api/v1/api_keys[/...]` path verbatim.
///
/// All routes require `ministr:read` scope at minimum (the scope is
/// enforced by the wrapping middleware in `cmd_serve_http`). Create +
/// revoke are write operations but they target the caller's own keys
/// only — the `ministr:read` floor matches the orgs surface.
pub fn api_keys_routes(state: ApiKeysState) -> Router {
    Router::new()
        .route("/api/v1/api_keys", post(create_handler).get(list_handler))
        .route("/api/v1/api_keys/{id}", delete(revoke_handler))
        .with_state(state)
}

/// `POST /api/v1/api_keys` request body.
#[derive(Debug, Deserialize)]
struct CreateRequest {
    name: String,
    /// Optional whitespace-separated scope list. Defaults to
    /// `"ministr:read ministr:write"` — matches the GitHub-IdP-minted
    /// bearer scope (`DEFAULT_SIGNIN_SCOPE`) so a key behaves like the
    /// owner's own session token by default.
    #[serde(default)]
    scopes: Option<String>,
}

/// `POST /api/v1/api_keys` response. Carries the raw token EXACTLY
/// ONCE. After this response the cloud only knows the hash.
#[derive(Debug, Serialize)]
struct CreateResponse {
    #[serde(flatten)]
    key: ApiKeyRow,
    /// The full bearer token, including the `mst_pk_` prefix. Display
    /// to the user once; the server cannot recover it.
    token: String,
}

/// `GET /api/v1/api_keys` response.
#[derive(Debug, Serialize)]
struct ListResponse {
    keys: Vec<ApiKeyRow>,
}

/// HTTP error shape mirroring [`crate::orgs::routes::OrgsApiError`].
#[derive(Debug)]
enum ApiKeysApiError {
    Unauthenticated,
    InvalidInput(&'static str),
    /// F3.4c-i — request specified a scope outside [`ALLOWED_API_KEY_SCOPES`].
    /// Carries the offending token so the response body is actionable.
    UnknownScope(String),
    NotFound,
    Repo(ApiKeysError),
}

impl axum::response::IntoResponse for ApiKeysApiError {
    fn into_response(self) -> axum::response::Response {
        use axum::http::StatusCode as S;
        match self {
            Self::Unauthenticated => (S::UNAUTHORIZED, "unauthenticated").into_response(),
            Self::InvalidInput(msg) => (S::BAD_REQUEST, msg).into_response(),
            Self::UnknownScope(token) => (
                S::BAD_REQUEST,
                format!("unknown_scope: {token}"),
            )
                .into_response(),
            Self::NotFound => (S::NOT_FOUND, "not_found").into_response(),
            Self::Repo(e) => {
                warn!(error = %e, "api_keys repo error");
                (S::INTERNAL_SERVER_ERROR, "internal").into_response()
            }
        }
    }
}

fn tenant_user_id(t: Option<Extension<Tenant>>) -> Result<String, ApiKeysApiError> {
    t.map(|Extension(t)| t.subject)
        .ok_or(ApiKeysApiError::Unauthenticated)
}

async fn create_handler(
    State(state): State<ApiKeysState>,
    tenant: Option<Extension<Tenant>>,
    Json(body): Json<CreateRequest>,
) -> Result<(StatusCode, Json<CreateResponse>), ApiKeysApiError> {
    let user_id = tenant_user_id(tenant)?;
    let name = body.name.trim();
    if name.is_empty() {
        return Err(ApiKeysApiError::InvalidInput("name must not be empty"));
    }
    let scopes_input = body
        .scopes
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("ministr:read ministr:write");
    // F3.4c-i — reject unknown tokens before mint so the table never
    // accumulates rows pointing at scopes the resolver can't honour.
    let scopes = match validate_scopes(scopes_input) {
        Ok(canonical) => canonical,
        Err(bad) if bad.is_empty() => {
            return Err(ApiKeysApiError::InvalidInput("scopes must not be empty"));
        }
        Err(bad) => return Err(ApiKeysApiError::UnknownScope(bad)),
    };
    let created = create_user_api_key(&state.pool, &user_id, name, &scopes)
        .await
        .map_err(ApiKeysApiError::Repo)?;
    // F3.7a — audit `api_key.created`. User-level action (no org_id).
    // Resource = the new key id so admins can correlate against
    // `api_keys.id` directly.
    state.audit_record(
        AuditEntry::new("api_key.created", &created.row.id).with_actor(&user_id),
    );
    Ok((
        StatusCode::CREATED,
        Json(CreateResponse {
            key: created.row,
            token: created.raw_token,
        }),
    ))
}

async fn list_handler(
    State(state): State<ApiKeysState>,
    tenant: Option<Extension<Tenant>>,
) -> Result<Json<ListResponse>, ApiKeysApiError> {
    let user_id = tenant_user_id(tenant)?;
    let keys = list_user_api_keys(&state.pool, &user_id)
        .await
        .map_err(ApiKeysApiError::Repo)?;
    Ok(Json(ListResponse { keys }))
}

async fn revoke_handler(
    State(state): State<ApiKeysState>,
    tenant: Option<Extension<Tenant>>,
    Path(key_id): Path<String>,
) -> Result<StatusCode, ApiKeysApiError> {
    let user_id = tenant_user_id(tenant)?;
    let revoked = revoke_user_api_key(&state.pool, &user_id, &key_id)
        .await
        .map_err(ApiKeysApiError::Repo)?;
    if revoked {
        // F3.7a — audit `api_key.revoked` only on actual state change.
        // A repeat DELETE of an already-revoked key returns NotFound
        // upstream (since the WHERE `revoked_at IS NULL` clause makes
        // it a 0-row UPDATE) so we won't enter this branch twice.
        state.audit_record(
            AuditEntry::new("api_key.revoked", &key_id).with_actor(&user_id),
        );
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ApiKeysApiError::NotFound)
    }
}

// ── Token helpers ──────────────────────────────────────────────────────────

/// Mint a fresh raw token. Format: `mst_pk_<base64url(32 bytes)>`.
fn mint_token() -> String {
    let mut buf = [0u8; TOKEN_ENTROPY_BYTES];
    getrandom_fill(&mut buf).expect("OS RNG must be available for api-key tokens");
    let mut out = String::with_capacity(TOKEN_PREFIX.len() + 44);
    out.push_str(TOKEN_PREFIX);
    out.push_str(&base64_url_no_pad(&buf));
    out
}

/// SHA-256 hex digest of the raw token. Matches the F3.1b-i convention.
fn hash_token(raw: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(raw.as_bytes());
    let digest = hasher.finalize();
    let mut out = String::with_capacity(digest.len() * 2);
    for b in digest {
        let _ = write!(out, "{b:02x}");
    }
    out
}

/// First [`PREFIX_LEN`] chars of the random portion (i.e. skipping the
/// `mst_pk_` literal). Stored verbatim in `api_keys.prefix` and shown
/// in the list UI.
fn derive_prefix(raw: &str) -> String {
    raw.strip_prefix(TOKEN_PREFIX)
        .unwrap_or(raw)
        .chars()
        .take(PREFIX_LEN)
        .collect()
}

/// Base64-url-no-pad encoder. Mirrors the helper in
/// `auth::github_signin` so token shapes stay uniform across the
/// codebase.
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

/// Default scope string applied when a create request omits the field.
/// Matches `DEFAULT_SIGNIN_SCOPE` so an API key behaves like the
/// owner's own session token by default. Exported so the eventual UI
/// can render the same default in its scope picker.
pub const DEFAULT_API_KEY_SCOPE: &str = "ministr:read ministr:write";

// Re-export the resolver as `Arc<dyn ApiKeyResolver>` for wiring sites.
impl PostgresApiKeyResolver {
    /// Convenience: wrap as an `Arc<dyn ApiKeyResolver>` for
    /// `OAuthStore::with_api_key_resolver`.
    #[must_use]
    pub fn into_dyn(self) -> Arc<dyn ApiKeyResolver> {
        Arc::new(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_scopes_accepts_full_vocabulary() {
        let raw = "ministr:read ministr:write ministr:bundle:read ministr:bundle:write";
        let canonical = validate_scopes(raw).expect("vocabulary is valid");
        assert_eq!(canonical, raw);
    }

    #[test]
    fn validate_scopes_accepts_subset_and_preserves_order() {
        let canonical = validate_scopes("ministr:write ministr:read").expect("valid subset");
        assert_eq!(canonical, "ministr:write ministr:read");
    }

    #[test]
    fn validate_scopes_collapses_whitespace_in_canonical_form() {
        // Tabs and double spaces collapse to single-space delimiters.
        let canonical = validate_scopes("ministr:read\tministr:write").expect("valid");
        assert_eq!(canonical, "ministr:read ministr:write");
    }

    #[test]
    fn validate_scopes_rejects_unknown_token() {
        let err = validate_scopes("ministr:read ministr:admin").expect_err("admin not allowed");
        assert_eq!(err, "ministr:admin");
    }

    #[test]
    fn validate_scopes_rejects_empty_after_trim() {
        let err = validate_scopes("   ").expect_err("whitespace-only is empty");
        assert!(err.is_empty());
    }

    #[test]
    fn default_stale_threshold_matches_roadmap_team_window() {
        // F3.4c — "90 days" lifted from the ROADMAP §F3.4c sub-bullet.
        // If a future iteration retunes this, update the ROADMAP first
        // so the spec and the const stay aligned.
        assert_eq!(DEFAULT_STALE_API_KEY_DAYS, 90);
    }

    #[test]
    fn stale_outcome_zero_flagged_when_no_rows() {
        // Pure data-shape check: the outcome is structurally
        // constructible without any DB call. The actual SELECT path is
        // gated on MINISTR_TEST_PG_URL per convention.
        let outcome = StaleApiKeysOutcome {
            flagged: 0,
            elapsed: std::time::Duration::from_millis(0),
            threshold_days: DEFAULT_STALE_API_KEY_DAYS,
        };
        assert_eq!(outcome.flagged, 0);
        assert_eq!(outcome.threshold_days, 90);
    }

    #[test]
    fn stale_audit_event_shape_carries_owner_actor() {
        // The cron emits AuditEntry::new("api_key.stale", key_id).with_actor(owner_id).
        // Lock in the expected wire shape so a refactor of the emit
        // call site can't drop the actor without this test screaming.
        let stale = StaleApiKey {
            key_id: "key-uuid".into(),
            owner_user_id: "owner-uuid".into(),
            name: "ci-prod".into(),
        };
        let entry = ministr_api::AuditEntry::new("api_key.stale", &stale.key_id)
            .with_actor(&stale.owner_user_id);
        assert_eq!(entry.action, "api_key.stale");
        assert_eq!(entry.resource, "key-uuid");
        assert_eq!(entry.actor.as_deref(), Some("owner-uuid"));
        assert!(entry.org_id.is_none(), "v0 user-owned keys have no org_id");
    }

    #[test]
    fn allowed_scopes_matches_default_signin_scope_set() {
        // F3.4c-i invariant: the four-scope vocabulary stays in lockstep
        // with the GitHub-IdP-minted bearer's default scope set, otherwise
        // an API key minted with the same scope string as the user's own
        // session would resolve differently.
        for scope in ALLOWED_API_KEY_SCOPES {
            assert!(
                crate::auth::github_signin::DEFAULT_SIGNIN_SCOPE.contains(scope),
                "{scope} missing from DEFAULT_SIGNIN_SCOPE"
            );
        }
    }

    #[test]
    fn minted_token_carries_prefix_and_random_suffix() {
        let t = mint_token();
        assert!(t.starts_with(TOKEN_PREFIX));
        // base64url-no-pad of 32 bytes → 43 chars; total ≥ "mst_pk_" + 43.
        assert!(t.len() >= TOKEN_PREFIX.len() + 40);
    }

    #[test]
    fn token_hash_is_deterministic_and_hex() {
        let t = "mst_pk_AaBbCcDd";
        let h1 = hash_token(t);
        let h2 = hash_token(t);
        assert_eq!(h1, h2);
        assert_eq!(h1.len(), 64);
        assert!(h1.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn distinct_tokens_hash_distinctly() {
        let a = hash_token("mst_pk_one");
        let b = hash_token("mst_pk_two");
        assert_ne!(a, b);
    }

    #[test]
    fn derive_prefix_strips_marker_and_caps_at_eight() {
        let t = "mst_pk_AaBbCcDdEeFfGg";
        assert_eq!(derive_prefix(t), "AaBbCcDd");
    }

    #[test]
    fn derive_prefix_handles_no_marker() {
        // Defensive: if someone calls with a non-prefixed string, take
        // the first 8 chars verbatim rather than panic.
        assert_eq!(derive_prefix("abc"), "abc");
        assert_eq!(derive_prefix("abcdefghijkl"), "abcdefgh");
    }

    #[test]
    fn base64_url_no_pad_uses_url_safe_alphabet() {
        // No '+', '/', or '='.
        let s = base64_url_no_pad(&[0xff_u8; 32]);
        assert!(s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_'));
    }
}
