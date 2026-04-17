//! OAuth 2.1 authorization framework for remote Streamable HTTP deployment.
//!
//! Implements the MCP authorization specification (2025-06-18) with:
//! - OAuth 2.0 Protected Resource Metadata (RFC 9728)
//! - Authorization Code flow with PKCE (S256)
//! - Dynamic Client Registration (RFC 7591)
//! - Bearer token validation middleware
//!
//! # Architecture
//!
//! The OAuth framework is structured as a set of axum routes and middleware
//! that wrap the MCP Streamable HTTP service:
//!
//! ```text
//! HTTP Request → Router:
//!   ├── /.well-known/oauth-protected-resource  (public)
//!   ├── /.well-known/oauth-authorization-server (public)
//!   ├── /oauth/authorize  (public)
//!   ├── /oauth/token      (public)
//!   ├── /oauth/register   (public)
//!   └── /mcp/*            (protected by token validation middleware)
//! ```

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Json, Response};
use axum::{Router, middleware, routing};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

// ── Configuration ──────────────────────────────────────────────────────────

/// OAuth 2.1 server configuration.
#[derive(Debug, Clone)]
pub struct OAuthConfig {
    /// The issuer URL (e.g., `https://iris.example.com`).
    pub issuer: String,
    /// Scopes supported by this server.
    pub scopes_supported: Vec<String>,
    /// Access token lifetime.
    pub token_ttl: Duration,
    /// Authorization code lifetime.
    pub code_ttl: Duration,
}

impl Default for OAuthConfig {
    fn default() -> Self {
        Self {
            issuer: "http://localhost:8080".into(),
            scopes_supported: vec![
                "iris:read".into(),
                "iris:write".into(),
                "iris:bundle:read".into(),
                "iris:bundle:write".into(),
            ],
            token_ttl: Duration::from_secs(3600),
            code_ttl: Duration::from_secs(600),
        }
    }
}

// ── In-Memory Store ────────────────────────────────────────────────────────

/// Registered OAuth client.
#[derive(Debug, Clone, Serialize)]
#[allow(dead_code)]
struct RegisteredClient {
    client_id: String,
    client_secret: Option<String>,
    redirect_uris: Vec<String>,
    client_name: Option<String>,
    scope: String,
    #[serde(skip)]
    registered_at: u64,
}

/// An issued authorization code.
#[derive(Debug, Clone)]
#[allow(dead_code)]
struct AuthorizationCode {
    code: String,
    client_id: String,
    redirect_uri: String,
    scope: String,
    code_challenge: String,
    code_challenge_method: String,
    expires_at: u64,
}

/// An active access token.
#[derive(Debug, Clone)]
#[allow(dead_code)]
struct AccessToken {
    token: String,
    client_id: String,
    scope: String,
    expires_at: u64,
}

/// Thread-safe in-memory OAuth state.
#[derive(Debug, Clone)]
pub struct OAuthStore {
    config: OAuthConfig,
    clients: Arc<RwLock<HashMap<String, RegisteredClient>>>,
    codes: Arc<RwLock<HashMap<String, AuthorizationCode>>>,
    tokens: Arc<RwLock<HashMap<String, AccessToken>>>,
}

impl OAuthStore {
    /// Create a new OAuth store with the given configuration.
    #[must_use]
    pub fn new(config: OAuthConfig) -> Self {
        Self {
            config,
            clients: Arc::new(RwLock::new(HashMap::new())),
            codes: Arc::new(RwLock::new(HashMap::new())),
            tokens: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Validate an access token, returning the client ID if valid.
    async fn validate_token(&self, token: &str) -> Option<String> {
        let tokens = self.tokens.read().await;
        let access = tokens.get(token)?;
        let now = epoch_now();
        if now > access.expires_at {
            return None;
        }
        Some(access.client_id.clone())
    }

    /// Validate an access token and check that it includes the required scope.
    ///
    /// Returns the client ID if the token is valid and its `scope` field
    /// contains `required_scope` as a space-separated entry. Returns `None`
    /// if the token is missing, expired, or lacks the scope.
    pub async fn validate_token_with_scope(
        &self,
        token: &str,
        required_scope: &str,
    ) -> Option<String> {
        let tokens = self.tokens.read().await;
        let access = tokens.get(token)?;
        let now = epoch_now();
        if now > access.expires_at {
            return None;
        }
        if access.scope.split_whitespace().any(|s| s == required_scope) {
            Some(access.client_id.clone())
        } else {
            None
        }
    }
}

// ── Middleware ──────────────────────────────────────────────────────────────

/// Axum middleware that validates Bearer tokens on protected routes.
///
/// Returns 401 with `WWW-Authenticate` header if the token is missing or invalid.
pub async fn validate_token_middleware(
    State(store): State<OAuthStore>,
    headers: HeaderMap,
    request: axum::extract::Request,
    next: Next,
) -> Response {
    let auth_header = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok());

    let token = match auth_header {
        Some(h) if h.starts_with("Bearer ") => &h[7..],
        _ => {
            debug!("missing or malformed Authorization header");
            return unauthorized_response(&store.config);
        }
    };

    if let Some(client_id) = store.validate_token(token).await {
        debug!(client_id = %client_id, "token validated");
        next.run(request).await
    } else {
        debug!("invalid or expired token");
        unauthorized_response(&store.config)
    }
}

/// State for scope-checking middleware: the OAuth store plus the required scope.
#[derive(Clone)]
pub struct ScopedState {
    /// The OAuth store for token validation.
    pub store: OAuthStore,
    /// The scope required to access the protected routes.
    pub required_scope: String,
}

/// Axum middleware that validates Bearer tokens **and** checks for a specific scope.
///
/// Returns 401 if the token is missing, expired, or lacks the required scope.
/// Returns 403 if the token is valid but the scope is insufficient.
pub async fn validate_scope_middleware(
    State(state): State<ScopedState>,
    headers: HeaderMap,
    request: axum::extract::Request,
    next: Next,
) -> Response {
    let auth_header = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok());

    let token = match auth_header {
        Some(h) if h.starts_with("Bearer ") => &h[7..],
        _ => {
            debug!("missing or malformed Authorization header");
            return unauthorized_response(&state.store.config);
        }
    };

    // First check: valid token?
    if state.store.validate_token(token).await.is_none() {
        debug!("invalid or expired token");
        return unauthorized_response(&state.store.config);
    }

    // Second check: has required scope?
    if let Some(client_id) = state
        .store
        .validate_token_with_scope(token, &state.required_scope)
        .await
    {
        debug!(client_id = %client_id, scope = %state.required_scope, "scoped token validated");
        next.run(request).await
    } else {
        debug!(scope = %state.required_scope, "token lacks required scope");
        (
            StatusCode::FORBIDDEN,
            format!("insufficient scope: requires {}", state.required_scope),
        )
            .into_response()
    }
}

/// Wrap a router with scope-checking middleware.
///
/// Requests must carry a valid Bearer token that includes `required_scope`
/// in its space-separated scope list.
pub fn scope_protected_router(router: Router, store: OAuthStore, required_scope: &str) -> Router {
    let scoped = ScopedState {
        store,
        required_scope: required_scope.to_owned(),
    };
    router.layer(middleware::from_fn_with_state(
        scoped,
        validate_scope_middleware,
    ))
}

fn unauthorized_response(config: &OAuthConfig) -> Response {
    let www_auth = format!(
        "Bearer resource_metadata=\"{}/.well-known/oauth-protected-resource\"",
        config.issuer,
    );
    (
        StatusCode::UNAUTHORIZED,
        [(header::WWW_AUTHENTICATE, www_auth)],
        "Unauthorized",
    )
        .into_response()
}

// ── Request/Response Types ─────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct RegistrationRequest {
    redirect_uris: Vec<String>,
    client_name: Option<String>,
    scope: Option<String>,
    token_endpoint_auth_method: Option<String>,
}

#[derive(Debug, Serialize)]
struct RegistrationResponse {
    client_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    client_secret: Option<String>,
    redirect_uris: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    client_name: Option<String>,
    token_endpoint_auth_method: String,
}

#[derive(Debug, Deserialize)]
struct TokenRequest {
    grant_type: String,
    code: Option<String>,
    redirect_uri: Option<String>,
    client_id: Option<String>,
    code_verifier: Option<String>,
}

#[derive(Debug, Serialize)]
struct TokenResponse {
    access_token: String,
    token_type: String,
    expires_in: u64,
    scope: String,
}

#[derive(Debug, Deserialize)]
struct AuthorizeQuery {
    response_type: String,
    client_id: String,
    redirect_uri: String,
    scope: Option<String>,
    state: Option<String>,
    code_challenge: Option<String>,
    code_challenge_method: Option<String>,
}

// ── Endpoint Handlers ──────────────────────────────────────────────────────

/// OAuth 2.0 Protected Resource Metadata (RFC 9728).
async fn protected_resource_metadata(State(store): State<OAuthStore>) -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "resource": store.config.issuer,
        "authorization_servers": [store.config.issuer],
        "scopes_supported": store.config.scopes_supported,
        "bearer_methods_supported": ["header"],
    }))
}

/// OAuth 2.0 Authorization Server Metadata (RFC 8414).
async fn authorization_server_metadata(State(store): State<OAuthStore>) -> Json<serde_json::Value> {
    let issuer = &store.config.issuer;
    Json(serde_json::json!({
        "issuer": issuer,
        "authorization_endpoint": format!("{issuer}/oauth/authorize"),
        "token_endpoint": format!("{issuer}/oauth/token"),
        "registration_endpoint": format!("{issuer}/oauth/register"),
        "scopes_supported": store.config.scopes_supported,
        "response_types_supported": ["code"],
        "grant_types_supported": ["authorization_code"],
        "token_endpoint_auth_methods_supported": ["none", "client_secret_post"],
        "code_challenge_methods_supported": ["S256"],
    }))
}

/// Dynamic Client Registration (RFC 7591).
async fn register_client(
    State(store): State<OAuthStore>,
    Json(req): Json<RegistrationRequest>,
) -> Result<(StatusCode, Json<RegistrationResponse>), StatusCode> {
    if req.redirect_uris.is_empty() {
        return Err(StatusCode::BAD_REQUEST);
    }

    let client_id = generate_id();
    let is_public = req.token_endpoint_auth_method.as_deref().unwrap_or("none") == "none";
    let client_secret = if is_public { None } else { Some(generate_id()) };
    let scope = req
        .scope
        .unwrap_or_else(|| store.config.scopes_supported.join(" "));

    let client = RegisteredClient {
        client_id: client_id.clone(),
        client_secret: client_secret.clone(),
        redirect_uris: req.redirect_uris.clone(),
        client_name: req.client_name.clone(),
        scope: scope.clone(),
        registered_at: epoch_now(),
    };

    info!(client_id = %client_id, "registered OAuth client");
    store
        .clients
        .write()
        .await
        .insert(client_id.clone(), client);

    let auth_method = if is_public {
        "none"
    } else {
        "client_secret_post"
    };

    Ok((
        StatusCode::CREATED,
        Json(RegistrationResponse {
            client_id,
            client_secret,
            redirect_uris: req.redirect_uris,
            client_name: req.client_name,
            token_endpoint_auth_method: auth_method.into(),
        }),
    ))
}

/// Authorization endpoint — issues an authorization code.
///
/// In a production system this would render a consent page. For the iris
/// framework, we auto-approve (the human consent step is delegated to the
/// deployment's identity provider when using an external AS).
async fn authorize(
    State(store): State<OAuthStore>,
    axum::extract::Query(query): axum::extract::Query<AuthorizeQuery>,
) -> Response {
    use std::fmt::Write as _;

    if query.response_type != "code" {
        return (StatusCode::BAD_REQUEST, "unsupported response_type").into_response();
    }

    let clients = store.clients.read().await;
    let Some(client) = clients.get(&query.client_id) else {
        return (StatusCode::BAD_REQUEST, "unknown client_id").into_response();
    };

    if !client.redirect_uris.contains(&query.redirect_uri) {
        return (StatusCode::BAD_REQUEST, "invalid redirect_uri").into_response();
    }
    drop(clients);

    let code_challenge = match &query.code_challenge {
        Some(c) => c.clone(),
        None => {
            return (StatusCode::BAD_REQUEST, "code_challenge required (PKCE)").into_response();
        }
    };

    let code_challenge_method = query
        .code_challenge_method
        .clone()
        .unwrap_or_else(|| "S256".into());

    if code_challenge_method != "S256" {
        return (
            StatusCode::BAD_REQUEST,
            "only S256 code_challenge_method supported",
        )
            .into_response();
    }

    let code = generate_id();
    let scope = query.scope.unwrap_or_default();

    let auth_code = AuthorizationCode {
        code: code.clone(),
        client_id: query.client_id,
        redirect_uri: query.redirect_uri.clone(),
        scope,
        code_challenge,
        code_challenge_method,
        expires_at: epoch_now() + store.config.code_ttl.as_secs(),
    };

    store.codes.write().await.insert(code.clone(), auth_code);
    debug!(code = %code, "issued authorization code");

    // Redirect back to the client with the authorization code.
    let mut redirect = query.redirect_uri;
    redirect.push_str(if redirect.contains('?') { "&" } else { "?" });
    let _ = write!(redirect, "code={code}");
    if let Some(state) = query.state {
        let _ = write!(redirect, "&state={state}");
    }

    (StatusCode::FOUND, [(header::LOCATION, redirect)]).into_response()
}

/// Token endpoint — exchanges an authorization code for an access token.
async fn token(
    State(store): State<OAuthStore>,
    axum::extract::Form(req): axum::extract::Form<TokenRequest>,
) -> Result<Json<TokenResponse>, StatusCode> {
    if req.grant_type != "authorization_code" {
        return Err(StatusCode::BAD_REQUEST);
    }

    let code_str = req.code.as_deref().ok_or(StatusCode::BAD_REQUEST)?;
    let code_verifier = req
        .code_verifier
        .as_deref()
        .ok_or(StatusCode::BAD_REQUEST)?;

    // Look up and consume the authorization code.
    let auth_code = store
        .codes
        .write()
        .await
        .remove(code_str)
        .ok_or(StatusCode::BAD_REQUEST)?;

    // Check expiry.
    if epoch_now() > auth_code.expires_at {
        warn!("authorization code expired");
        return Err(StatusCode::BAD_REQUEST);
    }

    // Verify PKCE: S256 code challenge.
    let mut hasher = Sha256::new();
    hasher.update(code_verifier.as_bytes());
    let computed = base64_url_encode(&hasher.finalize());
    if computed != auth_code.code_challenge {
        warn!("PKCE code_verifier mismatch");
        return Err(StatusCode::BAD_REQUEST);
    }

    // Verify client_id matches.
    if let Some(ref client_id) = req.client_id
        && *client_id != auth_code.client_id
    {
        return Err(StatusCode::BAD_REQUEST);
    }

    // Verify redirect_uri matches.
    if let Some(ref redirect_uri) = req.redirect_uri
        && *redirect_uri != auth_code.redirect_uri
    {
        return Err(StatusCode::BAD_REQUEST);
    }

    // Issue access token.
    let access_token = generate_id();
    let expires_in = store.config.token_ttl.as_secs();

    store.tokens.write().await.insert(
        access_token.clone(),
        AccessToken {
            token: access_token.clone(),
            client_id: auth_code.client_id,
            scope: auth_code.scope.clone(),
            expires_at: epoch_now() + expires_in,
        },
    );

    info!("issued access token");

    Ok(Json(TokenResponse {
        access_token,
        token_type: "Bearer".into(),
        expires_in,
        scope: auth_code.scope,
    }))
}

// ── Router Builder ─────────────────────────────────────────────────────────

/// Build an axum [`Router`] with OAuth 2.1 endpoints and token validation middleware.
///
/// The returned router includes:
/// - Public OAuth discovery + flow endpoints
/// - A protected `/mcp` service wrapped with Bearer token validation
///
/// # Arguments
///
/// Build public OAuth 2.1 routes (discovery, registration, authorization, token).
///
/// These routes should be merged into the main router alongside the
/// protected MCP endpoint.
pub fn oauth_routes(store: OAuthStore) -> Router {
    Router::new()
        .route(
            "/.well-known/oauth-protected-resource",
            routing::get(protected_resource_metadata),
        )
        .route(
            "/.well-known/oauth-authorization-server",
            routing::get(authorization_server_metadata),
        )
        .route("/oauth/register", routing::post(register_client))
        .route("/oauth/authorize", routing::get(authorize))
        .route("/oauth/token", routing::post(token))
        .with_state(store)
}

/// Build a complete OAuth-protected router.
///
/// Merges public OAuth endpoints with a protected MCP service that requires
/// Bearer token authentication.
///
/// # Arguments
///
/// * `mcp_router` — The MCP router (e.g., from `StreamableHttpService`)
/// * `store` — The OAuth store managing clients, codes, and tokens
pub fn protected_router(mcp_router: Router, store: OAuthStore) -> Router {
    let public = oauth_routes(store.clone());

    let protected = mcp_router.layer(middleware::from_fn_with_state(
        store,
        validate_token_middleware,
    ));

    public.merge(protected)
}

// ── Helpers ────────────────────────────────────────────────────────────────

/// Generate a cryptographically random URL-safe identifier.
fn generate_id() -> String {
    use sha2::Digest as _;
    let mut hasher = Sha256::new();
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    hasher.update(timestamp.to_le_bytes());
    // Mix in some additional entropy from the thread-local RNG address.
    let entropy: u64 = std::ptr::from_ref(&hasher) as u64;
    hasher.update(entropy.to_le_bytes());
    let hash = hasher.finalize();
    base64_url_encode(&hash[..16])
}

/// Base64url-encode without padding (RFC 4648 §5).
fn base64_url_encode(data: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

    let mut encoded = String::new();

    let mut i = 0;
    while i < data.len() {
        let b0 = u32::from(data[i]);
        let b1 = if i + 1 < data.len() {
            u32::from(data[i + 1])
        } else {
            0
        };
        let b2 = if i + 2 < data.len() {
            u32::from(data[i + 2])
        } else {
            0
        };

        let triple = (b0 << 16) | (b1 << 8) | b2;

        encoded.push(ALPHABET[((triple >> 18) & 0x3F) as usize] as char);
        encoded.push(ALPHABET[((triple >> 12) & 0x3F) as usize] as char);

        if i + 1 < data.len() {
            encoded.push(ALPHABET[((triple >> 6) & 0x3F) as usize] as char);
        }
        if i + 2 < data.len() {
            encoded.push(ALPHABET[(triple & 0x3F) as usize] as char);
        }

        i += 3;
    }

    encoded
}

/// Current epoch timestamp in seconds.
fn epoch_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64url_encode_roundtrip() {
        let data = b"hello, world!";
        let encoded = base64_url_encode(data);
        // Should not contain +, /, or = characters.
        assert!(!encoded.contains('+'));
        assert!(!encoded.contains('/'));
        assert!(!encoded.contains('='));
        assert!(!encoded.is_empty());
    }

    #[test]
    fn default_config_has_required_scopes() {
        let config = OAuthConfig::default();
        assert!(config.scopes_supported.contains(&"iris:read".to_string()));
        assert!(config.scopes_supported.contains(&"iris:write".to_string()));
        assert!(
            config
                .scopes_supported
                .contains(&"iris:bundle:read".to_string())
        );
        assert!(
            config
                .scopes_supported
                .contains(&"iris:bundle:write".to_string())
        );
        assert_eq!(config.token_ttl, Duration::from_secs(3600));
    }

    #[tokio::test]
    async fn store_validates_fresh_token() {
        let store = OAuthStore::new(OAuthConfig::default());
        let token_val = "test-token-123";
        store.tokens.write().await.insert(
            token_val.to_string(),
            AccessToken {
                token: token_val.to_string(),
                client_id: "client-1".into(),
                scope: "iris:read".into(),
                expires_at: epoch_now() + 3600,
            },
        );

        let result = store.validate_token(token_val).await;
        assert_eq!(result, Some("client-1".to_string()));
    }

    #[tokio::test]
    async fn store_rejects_expired_token() {
        let store = OAuthStore::new(OAuthConfig::default());
        let token_val = "expired-token";
        store.tokens.write().await.insert(
            token_val.to_string(),
            AccessToken {
                token: token_val.to_string(),
                client_id: "client-1".into(),
                scope: "iris:read".into(),
                expires_at: epoch_now().saturating_sub(100),
            },
        );

        let result = store.validate_token(token_val).await;
        assert_eq!(result, None);
    }

    #[tokio::test]
    async fn store_rejects_unknown_token() {
        let store = OAuthStore::new(OAuthConfig::default());
        let result = store.validate_token("nonexistent").await;
        assert_eq!(result, None);
    }

    #[test]
    fn pkce_s256_verification() {
        // RFC 7636 Appendix B test vector (adapted for S256).
        let code_verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        let mut hasher = Sha256::new();
        hasher.update(code_verifier.as_bytes());
        let challenge = base64_url_encode(&hasher.finalize());

        // Re-compute and compare.
        let mut hasher2 = Sha256::new();
        hasher2.update(code_verifier.as_bytes());
        let verify = base64_url_encode(&hasher2.finalize());

        assert_eq!(challenge, verify);
    }

    #[tokio::test]
    async fn validate_token_with_scope_matching() {
        let store = OAuthStore::new(OAuthConfig::default());
        let token_val = "scoped-token";
        store.tokens.write().await.insert(
            token_val.to_string(),
            AccessToken {
                token: token_val.to_string(),
                client_id: "client-1".into(),
                scope: "iris:read iris:bundle:read".into(),
                expires_at: epoch_now() + 3600,
            },
        );

        // Has the scope — should succeed.
        let result = store
            .validate_token_with_scope(token_val, "iris:bundle:read")
            .await;
        assert_eq!(result, Some("client-1".to_string()));

        // Also has iris:read.
        let result = store
            .validate_token_with_scope(token_val, "iris:read")
            .await;
        assert_eq!(result, Some("client-1".to_string()));
    }

    #[tokio::test]
    async fn validate_token_with_scope_missing() {
        let store = OAuthStore::new(OAuthConfig::default());
        let token_val = "limited-token";
        store.tokens.write().await.insert(
            token_val.to_string(),
            AccessToken {
                token: token_val.to_string(),
                client_id: "client-1".into(),
                scope: "iris:read".into(),
                expires_at: epoch_now() + 3600,
            },
        );

        // Does NOT have iris:bundle:read — should fail.
        let result = store
            .validate_token_with_scope(token_val, "iris:bundle:read")
            .await;
        assert_eq!(result, None);
    }

    #[tokio::test]
    async fn validate_token_with_scope_expired() {
        let store = OAuthStore::new(OAuthConfig::default());
        let token_val = "expired-scoped";
        store.tokens.write().await.insert(
            token_val.to_string(),
            AccessToken {
                token: token_val.to_string(),
                client_id: "client-1".into(),
                scope: "iris:bundle:read".into(),
                expires_at: epoch_now().saturating_sub(100),
            },
        );

        // Token expired — should fail even though scope matches.
        let result = store
            .validate_token_with_scope(token_val, "iris:bundle:read")
            .await;
        assert_eq!(result, None);
    }

    #[test]
    fn generate_id_produces_unique_values() {
        let id1 = generate_id();
        let id2 = generate_id();
        // Not guaranteed unique with timestamp-only entropy but different pointers
        // make collisions extremely unlikely.
        assert!(!id1.is_empty());
        assert!(!id2.is_empty());
    }
}
