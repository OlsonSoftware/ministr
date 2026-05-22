//! F5.2-b — OIDC (`OpenID` Connect) Service Provider browser-facing
//! endpoints.
//!
//! Mounts the per-org OIDC login-initiation route:
//!
//! - `GET /orgs/{id}/oidc/login` — loads the org's
//!   `org_oidc_configs` row, fetches the `IdP`'s OIDC discovery
//!   document (cached in-memory with ~1h TTL), builds an authorize
//!   URL with PKCE S256 + nonce + state, persists the
//!   `{state, nonce, pkce_verifier, org_id}` tuple in a pending-state
//!   map, and redirects (HTTP 302) the browser to the `IdP`'s
//!   `authorization_endpoint`.
//!
//! F5.2-c lands `GET /orgs/{id}/oidc/callback` which reads from the
//! pending-state map to validate the returned `state` and
//! complete the authorization-code → ID token exchange. F5.2-d
//! adds owner-gated CRUD endpoints for the config row.
//!
//! No `OAuth` gate on these routes — the `IdP` can't carry ministr
//! bearer tokens. Trust boundary is the discovery document + JWKS
//! verification, which happens at the callback (F5.2-c).
//!
//! Pure-Rust stack: openidconnect 4 + rustls + jsonwebtoken. No
//! libxmlsec1, no openssl-sys — sidesteps the F5.1-c-prep-libxmlsec-
//! crash entirely.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::Router;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use deadpool_postgres::Pool;
use openidconnect::core::{CoreClient, CoreProviderMetadata, CoreResponseType};
use openidconnect::reqwest as oidc_reqwest;
use openidconnect::{
    AuthenticationFlow, ClientId, ClientSecret, CsrfToken, IssuerUrl, Nonce, PkceCodeChallenge,
    PkceCodeVerifier, RedirectUrl, Scope,
};
use tokio::sync::RwLock;

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
        }
    }
}

/// Build the OIDC SP router. Mount on the application root
/// (`/orgs/{id}/oidc/login` lives outside the `OAuth`-protected
/// branch — the browser is the caller).
pub fn oidc_routes(state: OidcState) -> Router {
    Router::new()
        .route("/orgs/{id}/oidc/login", get(handle_login))
        .with_state(state)
}

/// One row from `org_oidc_configs`. Mirrors the schema in
/// migration 0011 with the subset of columns this handler needs.
struct OrgOidcConfig {
    issuer_url: String,
    client_id: String,
    client_secret: String,
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

    let redirect_uri = format!("http://localhost:8088/orgs/{org_id}/oidc/callback");
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
            "SELECT issuer_url, client_id, client_secret \
             FROM org_oidc_configs WHERE org_id = $1::text::uuid",
            &[&org_id.to_string()],
        )
        .await
        .map_err(|e| format!("query org_oidc_configs: {e:?}"))?;
    Ok(row.map(|r| OrgOidcConfig {
        issuer_url: r.get(0),
        client_id: r.get(1),
        client_secret: r.get(2),
    }))
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

fn internal_error(context: &str, e: &str) -> Response {
    tracing::warn!(context = %context, error = %e, "oidc endpoint error");
    (StatusCode::INTERNAL_SERVER_ERROR, "oidc internal error").into_response()
}
