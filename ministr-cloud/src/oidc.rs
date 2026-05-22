//! F5.2-b/c — OIDC (`OpenID` Connect) Service Provider browser-facing
//! endpoints.
//!
//! Mounts the per-org OIDC login + callback routes:
//!
//! - `GET /orgs/{id}/oidc/login` — loads the org's
//!   `org_oidc_configs` row, fetches the `IdP`'s OIDC discovery
//!   document (cached in-memory with ~1h TTL), builds an authorize
//!   URL with PKCE S256 + nonce + state, persists the
//!   `{state, nonce, pkce_verifier, org_id}` tuple in a pending-state
//!   map, and redirects (HTTP 302) the browser to the `IdP`'s
//!   `authorization_endpoint`.
//!
//! - `GET /orgs/{id}/oidc/callback` (F5.2-c) — consumes `?code=&state=`,
//!   exchanges the code at the `IdP`'s token endpoint with the saved
//!   PKCE verifier, validates the returned ID token (signature via
//!   JWKS, `iss` / `aud` / `nonce` claims, optional `email_verified`),
//!   extracts the email, upserts a `users` row (email-keyed; see
//!   [`crate::users::upsert_oidc_user`] for the v0 limitation), mints
//!   a bearer token via the same [`ministr_mcp::auth::OAuthStore`]
//!   the GitHub callback uses, and returns
//!   `{token, user_id, plan_id}` as JSON. Audit event `oidc.login`
//!   fires when an audit sink is wired.
//!
//! F5.2-d adds owner-gated CRUD endpoints for the config row.
//!
//! No `OAuth` gate on these routes — the `IdP` can't carry ministr
//! bearer tokens. Trust boundary is the discovery document + JWKS
//! verification, which happens at the callback.
//!
//! Pure-Rust stack: openidconnect 4 + rustls + jsonwebtoken. No
//! libxmlsec1, no openssl-sys — sidesteps the F5.1-c-prep-libxmlsec-
//! crash entirely.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::Router;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use deadpool_postgres::Pool;
use openidconnect::core::{CoreClient, CoreProviderMetadata, CoreResponseType};
use openidconnect::reqwest as oidc_reqwest;
use openidconnect::{
    AuthenticationFlow, AuthorizationCode, ClientId, ClientSecret, CsrfToken, IssuerUrl,
    Nonce, PkceCodeChallenge, PkceCodeVerifier, RedirectUrl, Scope,
};
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

use crate::users::upsert_oidc_user;

/// In-memory discovery cache TTL. OIDC providers rotate JWKS more
/// often than the discovery doc itself, so caching the metadata for
/// an hour is safe; openidconnect refetches JWKS independently at
/// token-verification time (F5.2-c).
const DISCOVERY_TTL: Duration = Duration::from_secs(3600);

/// In-memory pending-login TTL. Browsers typically complete OIDC
/// flows in under a minute; 10 minutes is generous and catches the
/// "user wandered off to read their email" case.
const PENDING_LOGIN_TTL: Duration = Duration::from_secs(600);

/// Per-route shared state. Holds the Postgres pool plus the two
/// in-memory caches (discovery + pending login state). Both caches
/// are owned by `Arc` so the state can be cloned freely into axum
/// route handlers.
#[derive(Clone)]
pub struct OidcState {
    pub pool: Arc<Pool>,
    /// `<issuer_url, (metadata, cached_at)>` — refetched when the
    /// entry is older than [`DISCOVERY_TTL`].
    discovery_cache: Arc<RwLock<HashMap<String, (CoreProviderMetadata, Instant)>>>,
    /// `<state_token, PendingLogin>` — populated on `/oidc/login`
    /// and consumed by `/oidc/callback` (F5.2-c).
    pending_logins: Arc<RwLock<HashMap<String, PendingLogin>>>,
    /// HTTP client used by openidconnect for discovery + token
    /// exchange. Single client per pod so connections + DNS are
    /// reused.
    http_client: oidc_reqwest::Client,
    /// F5.2-c — used to mint a bearer token after a successful
    /// ID-token validation. `None` on self-hosted / pre-F5.2-c
    /// deployments, in which case the callback returns 500 with a
    /// "callback not wired" message (the login endpoint also won't
    /// reach the callback without a configured org). `OAuthStore` is
    /// `Clone` (Arc-backed internally) so we hold it by value to
    /// match the `GitHubSigninState` pattern.
    oauth_store: Option<ministr_mcp::auth::OAuthStore>,
    /// F5.2-c — base URL used to construct the absolute
    /// `redirect_uri` passed to the `IdP`. Must match the value the
    /// `IdP` has registered for this Relying Party. `None` falls back
    /// to `http://localhost:8088` so the harness's old behaviour
    /// stays valid in tests; production deployments wire
    /// `MINISTR_CLOUD_BASE_URL` so the `IdP` sees the real public URL.
    cloud_base_url: Option<String>,
    /// F5.2-c — optional audit sink. `Some` fires an `oidc.login`
    /// audit row on successful callback. `None` (self-hosted serve
    /// or cloud deployments without audit wiring) skips emission.
    audit: Option<Arc<dyn ministr_api::AuditSink>>,
}

/// State persisted between `/oidc/login` and `/oidc/callback`. The
/// callback verifies that the returned `state` matches a pending
/// entry; the entry also carries the `nonce` (matched against the
/// ID token's `nonce` claim) and the PKCE `code_verifier` (used in
/// the token exchange).
#[allow(dead_code)] // fields are consumed by F5.2-c's callback handler
pub(crate) struct PendingLogin {
    pub(crate) org_id: String,
    pub(crate) nonce: Nonce,
    pub(crate) pkce_verifier: PkceCodeVerifier,
    pub(crate) created_at: Instant,
}

impl OidcState {
    /// Construct fresh state with an empty discovery cache + empty
    /// pending-login map. The HTTP client is built once with
    /// `redirect::Policy::none()` to avoid SSRF on discovery
    /// follow-redirects (per the openidconnect crate's recommended
    /// pattern).
    ///
    /// # Panics
    ///
    /// Panics if the reqwest client builder fails. The builder is
    /// configured with only `redirect::Policy::none()`, which has
    /// no failure mode in reqwest 0.13.
    #[must_use]
    pub fn new(pool: Arc<Pool>) -> Self {
        let http_client = oidc_reqwest::ClientBuilder::new()
            .redirect(oidc_reqwest::redirect::Policy::none())
            .build()
            .expect("oidc reqwest client builds with no options that can fail");
        Self {
            pool,
            discovery_cache: Arc::new(RwLock::new(HashMap::new())),
            pending_logins: Arc::new(RwLock::new(HashMap::new())),
            http_client,
            oauth_store: None,
            cloud_base_url: None,
            audit: None,
        }
    }

    /// F5.2-c — attach the bearer-token minter the callback uses on
    /// successful ID-token validation. Same store the GitHub callback
    /// uses; bearer tokens from either `IdP` are indistinguishable
    /// downstream because both flows mint via this single store.
    #[must_use]
    pub fn with_oauth_store(mut self, oauth_store: ministr_mcp::auth::OAuthStore) -> Self {
        self.oauth_store = Some(oauth_store);
        self
    }

    /// F5.2-c — set the cloud base URL used when assembling the
    /// `redirect_uri` parameter. The `IdP` must have this URL registered
    /// for the Relying Party. Trailing slashes are stripped.
    #[must_use]
    pub fn with_cloud_base_url(mut self, base_url: impl Into<String>) -> Self {
        let mut s = base_url.into();
        while s.ends_with('/') {
            s.pop();
        }
        self.cloud_base_url = Some(s);
        self
    }

    /// F5.2-c — wire an audit sink. When set, the callback emits an
    /// `oidc.login` row on successful ID-token validation.
    #[must_use]
    pub fn with_audit(mut self, audit: Arc<dyn ministr_api::AuditSink>) -> Self {
        self.audit = Some(audit);
        self
    }
}

/// Build the OIDC SP router. Mount on the application root
/// (`/orgs/{id}/oidc/login` lives outside the `OAuth`-protected
/// branch — the browser is the caller).
pub fn oidc_routes(state: OidcState) -> Router {
    Router::new()
        .route("/orgs/{id}/oidc/login", get(handle_login))
        .route("/orgs/{id}/oidc/callback", get(handle_callback))
        .with_state(state)
}

/// One row from `org_oidc_configs`. Mirrors the schema in
/// migration 0011 with the subset of columns these handlers need.
struct OrgOidcConfig {
    issuer_url: String,
    client_id: String,
    client_secret: String,
    enforce_email_verified: bool,
}

async fn handle_login(
    State(state): State<OidcState>,
    Path(org_id): Path<String>,
) -> Response {
    if parse_uuid(&org_id).is_none() {
        return bad_request_response("invalid org id");
    }
    let cfg = match load_config(&state, &org_id).await {
        Ok(Some(cfg)) => cfg,
        Ok(None) => return not_found_response(),
        Err(e) => return internal_error("load_config", &e),
    };

    let metadata = match get_or_fetch_discovery(&state, &cfg.issuer_url).await {
        Ok(m) => m,
        Err(e) => return internal_error("oidc discovery", &e),
    };

    let redirect_uri = build_redirect_uri(&state, &org_id);
    let redirect = match RedirectUrl::new(redirect_uri) {
        Ok(r) => r,
        Err(e) => return internal_error("invalid redirect_uri", &e.to_string()),
    };
    // openidconnect 4 uses typestate markers on Client's URL slots,
    // so inline the chain (set_redirect_uri changes the typestate
    // and we can't usefully return the resulting concrete type from
    // a helper without naming all 17 generic params).
    let client = CoreClient::from_provider_metadata(
        metadata,
        ClientId::new(cfg.client_id.clone()),
        Some(ClientSecret::new(cfg.client_secret.clone())),
    )
    .set_redirect_uri(redirect);

    let (pkce_challenge, pkce_verifier) = PkceCodeChallenge::new_random_sha256();
    let (auth_url, csrf, nonce) = client
        .authorize_url(
            AuthenticationFlow::<CoreResponseType>::AuthorizationCode,
            CsrfToken::new_random,
            Nonce::new_random,
        )
        .add_scope(Scope::new("openid".to_string()))
        .add_scope(Scope::new("email".to_string()))
        .add_scope(Scope::new("profile".to_string()))
        .set_pkce_challenge(pkce_challenge)
        .url();

    // Persist pending login keyed by the CSRF state token so the
    // F5.2-c callback can look it up.
    let state_token = csrf.secret().clone();
    {
        let mut pending = state.pending_logins.write().await;
        prune_expired(&mut pending);
        pending.insert(
            state_token,
            PendingLogin {
                org_id: org_id.clone(),
                nonce,
                pkce_verifier,
                created_at: Instant::now(),
            },
        );
    }

    redirect_to(auth_url.as_str())
}

/// Query string accepted by `/oidc/callback`. `code` and `state` are
/// the OIDC-standard params; `error` / `error_description` surface an
/// IdP-side rejection (the user denied the consent screen, etc.) and
/// pass through to the response so the browser can show something
/// useful.
#[derive(Debug, Deserialize)]
struct CallbackQuery {
    code: Option<String>,
    state: Option<String>,
    error: Option<String>,
    #[serde(rename = "error_description")]
    _error_description: Option<String>,
}

/// JSON body the harness + (eventually) the desktop loopback receiver
/// parse. F5.2-c v0 returns this directly; future iterations may
/// redirect to a `MINISTR_CLOUD_BASE_URL/auth/done?token=…` page so a
/// browser landing on the callback URL after a successful exchange
/// gets a friendly UI instead of raw JSON.
#[derive(Debug, Serialize)]
struct CallbackResponse {
    token: String,
    user_id: String,
    plan_id: String,
}

#[allow(clippy::too_many_lines)]
async fn handle_callback(
    State(state): State<OidcState>,
    Path(org_id): Path<String>,
    Query(query): Query<CallbackQuery>,
) -> Response {
    if parse_uuid(&org_id).is_none() {
        return bad_request_response("invalid org id");
    }

    // The IdP can reject the consent screen — `?error=` surfaces that
    // back to us. Surface it verbatim; the browser sees it and the
    // operator can debug. No bearer is minted in this branch.
    if let Some(err) = query.error {
        return bad_request_response_owned(format!("idp error: {err}"));
    }

    let Some(state_token) = query.state else {
        return bad_request_response("missing state");
    };
    let Some(code) = query.code else {
        return bad_request_response("missing code");
    };

    // Consume the pending entry up-front so a replayed callback can't
    // reuse the state. `remove` is the only mutation; everything else
    // operates on the owned `PendingLogin`.
    let pending = {
        let mut map = state.pending_logins.write().await;
        prune_expired(&mut map);
        map.remove(&state_token)
    };
    let Some(pending) = pending else {
        return bad_request_response("unknown or expired state");
    };
    if pending.org_id != org_id {
        // Replayed state from a different org's login flow. Reject
        // rather than risk crossing org boundaries.
        return bad_request_response("state belongs to a different org");
    }

    let Some(oauth_store) = state.oauth_store.as_ref() else {
        return internal_error(
            "oauth store",
            "OAuthStore not wired — cannot mint bearer token",
        );
    };

    let cfg = match load_config(&state, &org_id).await {
        Ok(Some(cfg)) => cfg,
        Ok(None) => return not_found_response(),
        Err(e) => return internal_error("load_config", &e),
    };

    let metadata = match get_or_fetch_discovery(&state, &cfg.issuer_url).await {
        Ok(m) => m,
        Err(e) => return internal_error("oidc discovery", &e),
    };

    let redirect_uri = build_redirect_uri(&state, &org_id);
    let redirect = match RedirectUrl::new(redirect_uri) {
        Ok(r) => r,
        Err(e) => return internal_error("invalid redirect_uri", &e.to_string()),
    };
    let client = CoreClient::from_provider_metadata(
        metadata,
        ClientId::new(cfg.client_id.clone()),
        Some(ClientSecret::new(cfg.client_secret.clone())),
    )
    .set_redirect_uri(redirect);

    // Token exchange: code → access_token + id_token. The PKCE
    // verifier proves we're the same client that initiated the flow.
    let token_request = match client.exchange_code(AuthorizationCode::new(code)) {
        Ok(req) => req.set_pkce_verifier(pending.pkce_verifier),
        Err(e) => return internal_error("build token request", &e.to_string()),
    };
    let token_response = match token_request.request_async(&state.http_client).await {
        Ok(t) => t,
        Err(e) => return internal_error("token exchange", &e.to_string()),
    };

    let Some(id_token) = token_response.extra_fields().id_token() else {
        return internal_error(
            "id_token missing",
            "token response carried no id_token (IdP misconfigured?)",
        );
    };

    // Validate the ID token: signature against the IdP's JWKS, `iss`
    // matches the discovery doc's issuer, `aud` matches our client_id,
    // `nonce` matches the value we minted at /oidc/login. The verifier
    // pulls all of those from the client; we only have to supply the
    // saved nonce.
    let claims = match id_token.claims(&client.id_token_verifier(), &pending.nonce) {
        Ok(c) => c,
        Err(e) => return bad_request_response_owned(format!("id_token validation: {e}")),
    };

    // Extract email — OIDC's `email` scope is the standard channel.
    // For enterprise customers the IdP MUST issue it; without it we
    // can't key the users table or grant access.
    let Some(email_claim) = claims.email() else {
        return bad_request_response(
            "id_token carries no email claim — request the `email` scope or configure the IdP",
        );
    };
    let email = email_claim.as_str();

    // Honour `enforce_email_verified` — most enterprise IdPs always
    // set `email_verified=true` so the default (TRUE in the schema)
    // is the safe choice. Customers running a non-conforming IdP can
    // flip it off explicitly via the F5.2-d CRUD path.
    if cfg.enforce_email_verified
        && !claims.email_verified().unwrap_or(false)
    {
        return bad_request_response(
            "id_token's email_verified claim is false — sign-in rejected",
        );
    }

    let user = match upsert_oidc_user(&state.pool, email).await {
        Ok(u) => u,
        Err(e) => return internal_error("upsert_oidc_user", &e.to_string()),
    };

    let token = match oauth_store
        .issue_bearer_token(&user.id, DEFAULT_OIDC_SIGNIN_SCOPE)
        .await
    {
        Ok(t) => t,
        Err(e) => return internal_error("issue_bearer_token", &e.to_string()),
    };

    // F3.7-style audit emission. Best-effort: sink failures are logged
    // by the impl, never propagated.
    if let Some(audit) = state.audit.as_ref() {
        audit.record(
            ministr_api::AuditEntry::new("oidc.login", &user.id)
                .with_org(&org_id)
                .with_actor(&user.id),
        );
    }

    tracing::info!(
        user_id = %user.id,
        org_id = %org_id,
        issuer = %cfg.issuer_url,
        inserted = user.inserted,
        "oidc sign-in completed; bearer token issued"
    );

    let body = CallbackResponse {
        token,
        user_id: user.id,
        plan_id: user.plan_id,
    };
    (StatusCode::OK, axum::Json(body)).into_response()
}

/// Default scope minted for OIDC sign-ins. Mirrors
/// `ministr_mcp::auth::DEFAULT_SIGNIN_SCOPE` so the federated bearer
/// is indistinguishable from a GitHub-IdP bearer downstream. Kept as
/// a constant in this module rather than re-exporting so a future
/// per-tier scope tightening (Pro vs Team minting different scopes)
/// stays scoped to OIDC.
const DEFAULT_OIDC_SIGNIN_SCOPE: &str = "ministr:read ministr:write";

async fn load_config(
    state: &OidcState,
    org_id: &str,
) -> Result<Option<OrgOidcConfig>, String> {
    let client = state
        .pool
        .get()
        .await
        .map_err(|e| format!("pool get: {e}"))?;
    let row = client
        .query_opt(
            "SELECT issuer_url, client_id, client_secret, enforce_email_verified \
             FROM org_oidc_configs WHERE org_id = $1::text::uuid",
            &[&org_id.to_string()],
        )
        .await
        .map_err(|e| format!("query org_oidc_configs: {e:?}"))?;
    Ok(row.map(|r| OrgOidcConfig {
        issuer_url: r.get(0),
        client_id: r.get(1),
        client_secret: r.get(2),
        enforce_email_verified: r.get(3),
    }))
}

/// Build the per-org callback URL from the cloud base URL configured
/// on the state. The `IdP` must have this URL registered for the
/// Relying Party (per OIDC spec §3.1.2.1).
fn build_redirect_uri(state: &OidcState, org_id: &str) -> String {
    let base = state
        .cloud_base_url
        .as_deref()
        .unwrap_or("http://localhost:8088");
    format!("{base}/orgs/{org_id}/oidc/callback")
}

async fn get_or_fetch_discovery(
    state: &OidcState,
    issuer_url: &str,
) -> Result<CoreProviderMetadata, String> {
    // Cache hit fast-path.
    {
        let cache = state.discovery_cache.read().await;
        if let Some((metadata, cached_at)) = cache.get(issuer_url)
            && cached_at.elapsed() < DISCOVERY_TTL
        {
            return Ok(metadata.clone());
        }
    }

    let issuer = IssuerUrl::new(issuer_url.to_string())
        .map_err(|e| format!("invalid issuer_url: {e}"))?;
    let metadata = CoreProviderMetadata::discover_async(issuer, &state.http_client)
        .await
        .map_err(|e| format!("discover_async: {e}"))?;

    let mut cache = state.discovery_cache.write().await;
    cache.insert(issuer_url.to_string(), (metadata.clone(), Instant::now()));
    Ok(metadata)
}

/// Drop pending-login entries older than [`PENDING_LOGIN_TTL`].
/// Called on insert so the map doesn't grow unbounded; F5.2-c's
/// callback also evicts the consumed entry.
fn prune_expired(pending: &mut HashMap<String, PendingLogin>) {
    pending.retain(|_, v| v.created_at.elapsed() < PENDING_LOGIN_TTL);
}

/// Minimal UUID v4 string validation; same shape as
/// `crate::saml::parse_uuid` (duplicated here to keep the module
/// self-contained — both modules will pull a shared helper if a
/// third site adds the check).
fn parse_uuid(s: &str) -> Option<String> {
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
    Some(s.to_string())
}

fn redirect_to(url: &str) -> Response {
    let mut headers = HeaderMap::new();
    if let Ok(v) = HeaderValue::from_str(url) {
        headers.insert(header::LOCATION, v);
    }
    (StatusCode::FOUND, headers, "").into_response()
}

fn not_found_response() -> Response {
    (StatusCode::NOT_FOUND, "oidc config not found for org").into_response()
}

fn bad_request_response(msg: &'static str) -> Response {
    (StatusCode::BAD_REQUEST, msg).into_response()
}

fn bad_request_response_owned(msg: String) -> Response {
    (StatusCode::BAD_REQUEST, msg).into_response()
}

fn internal_error(context: &str, e: &str) -> Response {
    tracing::warn!(context = %context, error = %e, "oidc endpoint error");
    (StatusCode::INTERNAL_SERVER_ERROR, "oidc internal error").into_response()
}

// ─── F5.2-d — per-org OIDC config CRUD ────────────────────────────────
//
// Three routes mounted at `/api/v1/orgs/{id}/oidc/config`. Same
// shape as the F5.1-d SAML config CRUD: owner-only via
// `assert_owner_or_admin`, upsert via `ON CONFLICT (org_id)`, GET
// returns the row but redacts the `client_secret` (OIDC's only
// bearer-grade material), DELETE removes the row + returns 204.
//
// The route handlers reuse [`OidcState`]'s `pool` field; no new
// state struct needed. Mount with `scope_protected_router` behind
// `ministr:read` so the [`ministr_mcp::auth::tenant::Tenant`]
// extension is present when the ACL fires.

/// Sentinel string returned in place of the real `client_secret` on
/// every GET response. Choosing a sentinel rather than omitting the
/// field keeps the wire shape stable so frontend code can detect
/// "config exists" without a separate HEAD call.
pub const REDACTED_CLIENT_SECRET: &str = "[REDACTED]";

/// F5.2-d — per-org OIDC config CRUD router. Mount under the
/// `OAuth`-protected branch in `cmd_serve_http`; owner-only ACL is
/// enforced by each handler via [`assert_oidc_owner_or_admin`].
pub fn oidc_config_routes(state: OidcState) -> Router {
    use axum::routing::post;
    Router::new()
        .route(
            "/api/v1/orgs/{id}/oidc/config",
            post(handle_oidc_config_upsert)
                .get(handle_oidc_config_get)
                .delete(handle_oidc_config_delete),
        )
        .with_state(state)
}

#[derive(Debug)]
enum OidcConfigError {
    Unauthenticated,
    Forbidden,
    NotFound,
    Invalid(&'static str),
    Db(String),
}

impl axum::response::IntoResponse for OidcConfigError {
    fn into_response(self) -> Response {
        match self {
            Self::Unauthenticated => {
                (StatusCode::UNAUTHORIZED, "unauthenticated").into_response()
            }
            Self::Forbidden => (StatusCode::FORBIDDEN, "forbidden").into_response(),
            Self::NotFound => (StatusCode::NOT_FOUND, "not_found").into_response(),
            Self::Invalid(msg) => (StatusCode::BAD_REQUEST, msg).into_response(),
            Self::Db(msg) => {
                tracing::warn!(error = %msg, "oidc config db error");
                (StatusCode::INTERNAL_SERVER_ERROR, "internal").into_response()
            }
        }
    }
}

/// POST body for `/api/v1/orgs/{id}/oidc/config`. `issuer_url`,
/// `client_id`, and `client_secret` are required; the three claim-
/// mapping fields + `enforce_email_verified` are optional and fall
/// back to the table defaults (`groups` / `email` / `name` / true)
/// when absent.
#[derive(serde::Deserialize)]
struct OidcConfigUpsertBody {
    issuer_url: String,
    client_id: String,
    client_secret: String,
    #[serde(default)]
    groups_claim: Option<String>,
    #[serde(default)]
    email_claim: Option<String>,
    #[serde(default)]
    name_claim: Option<String>,
    #[serde(default)]
    enforce_email_verified: Option<bool>,
}

/// GET / upsert response shape. `client_secret` is always
/// [`REDACTED_CLIENT_SECRET`] — the only writer is the upsert
/// handler and the harness's direct INSERT path; reads never expose
/// it.
#[derive(serde::Serialize)]
struct OidcConfigView {
    org_id: String,
    issuer_url: String,
    client_id: String,
    client_secret: String,
    groups_claim: String,
    email_claim: String,
    name_claim: String,
    enforce_email_verified: bool,
    created_at: String,
    updated_at: String,
}

/// Owner / admin ACL, identical shape to [`crate::saml`]'s helper.
/// Duplicated rather than shared because both modules want their own
/// `*ConfigError` variants; the helper's body is two lines.
async fn assert_oidc_owner_or_admin(
    pool: &Pool,
    org_id: &str,
    user_id: &str,
) -> Result<(), OidcConfigError> {
    let role = crate::orgs::repo::member_role(pool, org_id, user_id)
        .await
        .map_err(|e| OidcConfigError::Db(format!("member_role: {e}")))?;
    if !matches!(role.as_deref(), Some("owner" | "admin")) {
        return Err(OidcConfigError::Forbidden);
    }
    Ok(())
}

fn validate_oidc_upsert(body: &OidcConfigUpsertBody) -> Result<(), OidcConfigError> {
    if body.issuer_url.trim().is_empty() {
        return Err(OidcConfigError::Invalid("issuer_url is required"));
    }
    if body.client_id.trim().is_empty() {
        return Err(OidcConfigError::Invalid("client_id is required"));
    }
    if body.client_secret.trim().is_empty() {
        return Err(OidcConfigError::Invalid("client_secret is required"));
    }
    // Cheap sanity check on the issuer URL — full OIDC discovery
    // validates the URL when /oidc/login runs. Reject obvious
    // misconfiguration (missing scheme) here so the owner sees the
    // error at config-time rather than at first-sign-in.
    if !body.issuer_url.starts_with("http://") && !body.issuer_url.starts_with("https://") {
        return Err(OidcConfigError::Invalid(
            "issuer_url must start with http:// or https://",
        ));
    }
    Ok(())
}

#[allow(clippy::too_many_lines)]
async fn handle_oidc_config_upsert(
    State(state): State<OidcState>,
    tenant: Option<axum::Extension<ministr_mcp::auth::tenant::Tenant>>,
    Path(org_id): Path<String>,
    axum::Json(body): axum::Json<OidcConfigUpsertBody>,
) -> Result<(StatusCode, axum::Json<OidcConfigView>), OidcConfigError> {
    let tenant = tenant.ok_or(OidcConfigError::Unauthenticated)?;
    if parse_uuid(&org_id).is_none() {
        return Err(OidcConfigError::Invalid("invalid org id"));
    }
    assert_oidc_owner_or_admin(&state.pool, &org_id, &tenant.0.subject).await?;
    validate_oidc_upsert(&body)?;

    let groups_claim = body.groups_claim.as_deref().unwrap_or("groups");
    let email_claim = body.email_claim.as_deref().unwrap_or("email");
    let name_claim = body.name_claim.as_deref().unwrap_or("name");
    let enforce = body.enforce_email_verified.unwrap_or(true);

    let client = state
        .pool
        .get()
        .await
        .map_err(|e| OidcConfigError::Db(format!("pool get: {e}")))?;
    let row = client
        .query_one(
            "INSERT INTO org_oidc_configs (\
                org_id, issuer_url, client_id, client_secret, \
                groups_claim, email_claim, name_claim, enforce_email_verified) \
             VALUES ($1::text::uuid, $2, $3, $4, $5, $6, $7, $8) \
             ON CONFLICT (org_id) DO UPDATE SET \
                issuer_url = EXCLUDED.issuer_url, \
                client_id = EXCLUDED.client_id, \
                client_secret = EXCLUDED.client_secret, \
                groups_claim = EXCLUDED.groups_claim, \
                email_claim = EXCLUDED.email_claim, \
                name_claim = EXCLUDED.name_claim, \
                enforce_email_verified = EXCLUDED.enforce_email_verified, \
                updated_at = NOW() \
             RETURNING issuer_url, client_id, groups_claim, email_claim, \
                       name_claim, enforce_email_verified, \
                       to_char(created_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"'), \
                       to_char(updated_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"')",
            &[
                &org_id,
                &body.issuer_url,
                &body.client_id,
                &body.client_secret,
                &groups_claim,
                &email_claim,
                &name_claim,
                &enforce,
            ],
        )
        .await
        .map_err(|e| OidcConfigError::Db(format!("upsert: {e:?}")))?;

    // `client_secret` is intentionally NOT in the RETURNING clause —
    // even an inadvertent log of `row` can't leak it. Hardcoding the
    // sentinel in the view here makes the redaction visible at the
    // handler site.
    let view = OidcConfigView {
        org_id: org_id.clone(),
        issuer_url: row.get(0),
        client_id: row.get(1),
        client_secret: REDACTED_CLIENT_SECRET.to_string(),
        groups_claim: row.get(2),
        email_claim: row.get(3),
        name_claim: row.get(4),
        enforce_email_verified: row.get(5),
        created_at: row.get(6),
        updated_at: row.get(7),
    };

    // Bust the per-org pending-login cache so the next /oidc/login
    // call rebuilds discovery against the new issuer (rather than
    // surfacing the old IdP that the operator just rotated away).
    // The discovery cache is keyed by issuer_url so changing
    // issuer_url already misses; the pending_logins map TTL-evicts
    // on its own. Both are conservative — no explicit invalidation
    // needed here.
    let _ = state;

    Ok((StatusCode::OK, axum::Json(view)))
}

async fn handle_oidc_config_get(
    State(state): State<OidcState>,
    tenant: Option<axum::Extension<ministr_mcp::auth::tenant::Tenant>>,
    Path(org_id): Path<String>,
) -> Result<axum::Json<OidcConfigView>, OidcConfigError> {
    let tenant = tenant.ok_or(OidcConfigError::Unauthenticated)?;
    if parse_uuid(&org_id).is_none() {
        return Err(OidcConfigError::Invalid("invalid org id"));
    }
    assert_oidc_owner_or_admin(&state.pool, &org_id, &tenant.0.subject).await?;

    let client = state
        .pool
        .get()
        .await
        .map_err(|e| OidcConfigError::Db(format!("pool get: {e}")))?;
    let row = client
        .query_opt(
            "SELECT issuer_url, client_id, groups_claim, email_claim, \
                    name_claim, enforce_email_verified, \
                    to_char(created_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"'), \
                    to_char(updated_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"') \
             FROM org_oidc_configs WHERE org_id = $1::text::uuid",
            &[&org_id],
        )
        .await
        .map_err(|e| OidcConfigError::Db(format!("select: {e:?}")))?
        .ok_or(OidcConfigError::NotFound)?;

    Ok(axum::Json(OidcConfigView {
        org_id: org_id.clone(),
        issuer_url: row.get(0),
        client_id: row.get(1),
        client_secret: REDACTED_CLIENT_SECRET.to_string(),
        groups_claim: row.get(2),
        email_claim: row.get(3),
        name_claim: row.get(4),
        enforce_email_verified: row.get(5),
        created_at: row.get(6),
        updated_at: row.get(7),
    }))
}

async fn handle_oidc_config_delete(
    State(state): State<OidcState>,
    tenant: Option<axum::Extension<ministr_mcp::auth::tenant::Tenant>>,
    Path(org_id): Path<String>,
) -> Result<StatusCode, OidcConfigError> {
    let tenant = tenant.ok_or(OidcConfigError::Unauthenticated)?;
    if parse_uuid(&org_id).is_none() {
        return Err(OidcConfigError::Invalid("invalid org id"));
    }
    assert_oidc_owner_or_admin(&state.pool, &org_id, &tenant.0.subject).await?;

    let client = state
        .pool
        .get()
        .await
        .map_err(|e| OidcConfigError::Db(format!("pool get: {e}")))?;
    let deleted = client
        .execute(
            "DELETE FROM org_oidc_configs WHERE org_id = $1::text::uuid",
            &[&org_id],
        )
        .await
        .map_err(|e| OidcConfigError::Db(format!("delete: {e:?}")))?;
    if deleted == 0 {
        return Err(OidcConfigError::NotFound);
    }
    Ok(StatusCode::NO_CONTENT)
}
