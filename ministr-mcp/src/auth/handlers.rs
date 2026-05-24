//! OAuth 2.1 endpoint handlers.
//!
//! Each function corresponds to a single route. Storage I/O goes through
//! `OAuthStore`; PKCE/scope/redirect-URI policy lives here. Request/response
//! DTOs are local to this file.

use std::fmt::Write as _;

use axum::extract::{Form, Query, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Json, Response};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tracing::{info, warn};

use super::store::OAuthStore;
use super::types::{AccessToken, AuthorizationCode, RegisteredClient};
use super::util::{base64_url_encode, epoch_now, generate_id};

// ── Request / Response DTOs ────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub(super) struct RegistrationRequest {
    redirect_uris: Vec<String>,
    client_name: Option<String>,
    scope: Option<String>,
    token_endpoint_auth_method: Option<String>,
    // SEP-837 — clients declare their type during DCR. Accepted but
    // not acted upon (we treat all clients identically).
    #[allow(dead_code)]
    application_type: Option<String>,
}

#[derive(Debug, Serialize)]
pub(super) struct RegistrationResponse {
    client_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    client_secret: Option<String>,
    redirect_uris: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    client_name: Option<String>,
    token_endpoint_auth_method: String,
}

#[derive(Debug, Deserialize)]
pub(super) struct TokenRequest {
    grant_type: String,
    code: Option<String>,
    redirect_uri: Option<String>,
    client_id: Option<String>,
    code_verifier: Option<String>,
}

#[derive(Debug, Serialize)]
pub(super) struct TokenResponse {
    access_token: String,
    token_type: String,
    expires_in: u64,
    scope: String,
}

#[derive(Debug, Deserialize)]
pub(super) struct AuthorizeQuery {
    response_type: String,
    client_id: String,
    redirect_uri: String,
    scope: Option<String>,
    state: Option<String>,
    code_challenge: Option<String>,
    code_challenge_method: Option<String>,
}

// ── Metadata endpoints ─────────────────────────────────────────────────────

/// RFC 9728 — protected resource metadata.
pub(super) async fn protected_resource_metadata(
    State(store): State<OAuthStore>,
) -> Json<serde_json::Value> {
    let config = store.config();
    Json(serde_json::json!({
        "resource": config.issuer,
        "authorization_servers": [config.issuer],
        "scopes_supported": config.scopes_supported,
        "bearer_methods_supported": ["header"],
    }))
}

/// RFC 8414 — authorization server metadata.
pub(super) async fn authorization_server_metadata(
    State(store): State<OAuthStore>,
) -> Json<serde_json::Value> {
    let config = store.config();
    let issuer = &config.issuer;
    Json(serde_json::json!({
        "issuer": issuer,
        "authorization_endpoint": format!("{issuer}/oauth/authorize"),
        "token_endpoint": format!("{issuer}/oauth/token"),
        "registration_endpoint": format!("{issuer}/oauth/register"),
        "scopes_supported": config.scopes_supported,
        "response_types_supported": ["code"],
        "grant_types_supported": ["authorization_code"],
        "token_endpoint_auth_methods_supported": ["none", "client_secret_post"],
        "code_challenge_methods_supported": ["S256"],
    }))
}

// ── Dynamic Client Registration ────────────────────────────────────────────

/// RFC 7591 dynamic client registration.
pub(super) async fn register_client(
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
        .unwrap_or_else(|| store.config().scopes_supported.join(" "));

    let client = RegisteredClient {
        client_id: client_id.clone(),
        client_secret: client_secret.clone(),
        redirect_uris: req.redirect_uris.clone(),
        client_name: req.client_name.clone(),
        scope: scope.clone(),
        registered_at: epoch_now(),
    };

    if let Err(e) = store.save_client(client).await {
        warn!(error = %e, "failed to persist registered client");
        return Err(StatusCode::INTERNAL_SERVER_ERROR);
    }
    info!(client_id = %client_id, "registered OAuth client");

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

// ── Authorize ──────────────────────────────────────────────────────────────

/// Authorization endpoint — issues an authorization code.
///
/// Auto-approves: human consent is delegated to the deployment's identity
/// provider when an external authorization server is configured.
pub(super) async fn authorize(
    State(store): State<OAuthStore>,
    Query(query): Query<AuthorizeQuery>,
) -> Response {
    if query.response_type != "code" {
        return (StatusCode::BAD_REQUEST, "unsupported response_type").into_response();
    }

    let client = match store.get_client(&query.client_id).await {
        Ok(Some(c)) => c,
        Ok(None) => return (StatusCode::BAD_REQUEST, "unknown client_id").into_response(),
        Err(e) => {
            warn!(error = %e, "client lookup failed");
            return (StatusCode::INTERNAL_SERVER_ERROR, "storage unavailable").into_response();
        }
    };

    if !client.redirect_uris.contains(&query.redirect_uri) {
        return (StatusCode::BAD_REQUEST, "invalid redirect_uri").into_response();
    }

    let Some(code_challenge) = query.code_challenge.clone() else {
        return (StatusCode::BAD_REQUEST, "code_challenge required (PKCE)").into_response();
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
        expires_at: epoch_now() + store.config().code_ttl.as_secs(),
    };

    if let Err(e) = store.save_code(auth_code).await {
        warn!(error = %e, "failed to persist authorization code");
        return (StatusCode::INTERNAL_SERVER_ERROR, "storage unavailable").into_response();
    }

    let mut redirect = query.redirect_uri;
    redirect.push_str(if redirect.contains('?') { "&" } else { "?" });
    let _ = write!(redirect, "code={code}");
    if let Some(state) = query.state {
        let _ = write!(redirect, "&state={state}");
    }
    // SEP-2468 / RFC 9207 — include the issuer so clients can validate
    // the authorization response came from the expected server.
    let issuer = &store.config().issuer;
    let mut encoded_issuer = String::with_capacity(issuer.len());
    for b in issuer.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~' | b':' | b'/') {
            encoded_issuer.push(b as char);
        } else {
            let _ = write!(encoded_issuer, "%{b:02X}");
        }
    }
    let _ = write!(redirect, "&iss={encoded_issuer}");

    (StatusCode::FOUND, [(header::LOCATION, redirect)]).into_response()
}

// ── Token ──────────────────────────────────────────────────────────────────

/// Token endpoint — exchanges an authorization code for an access token.
pub(super) async fn token(
    State(store): State<OAuthStore>,
    Form(req): Form<TokenRequest>,
) -> Result<Json<TokenResponse>, StatusCode> {
    if req.grant_type != "authorization_code" {
        return Err(StatusCode::BAD_REQUEST);
    }

    let code_str = req.code.as_deref().ok_or(StatusCode::BAD_REQUEST)?;
    let code_verifier = req
        .code_verifier
        .as_deref()
        .ok_or(StatusCode::BAD_REQUEST)?;

    let auth_code = match store.take_code(code_str).await {
        Ok(Some(c)) => c,
        Ok(None) => return Err(StatusCode::BAD_REQUEST),
        Err(e) => {
            warn!(error = %e, "code lookup failed");
            return Err(StatusCode::INTERNAL_SERVER_ERROR);
        }
    };

    if epoch_now() > auth_code.expires_at {
        warn!("authorization code expired");
        return Err(StatusCode::BAD_REQUEST);
    }

    // PKCE S256 verification.
    let mut hasher = Sha256::new();
    hasher.update(code_verifier.as_bytes());
    let computed = base64_url_encode(&hasher.finalize());
    if computed != auth_code.code_challenge {
        warn!("PKCE code_verifier mismatch");
        return Err(StatusCode::BAD_REQUEST);
    }

    if let Some(ref client_id) = req.client_id
        && *client_id != auth_code.client_id
    {
        return Err(StatusCode::BAD_REQUEST);
    }

    if let Some(ref redirect_uri) = req.redirect_uri
        && *redirect_uri != auth_code.redirect_uri
    {
        return Err(StatusCode::BAD_REQUEST);
    }

    let access_token = generate_id();
    let expires_in = store.config().token_ttl.as_secs();

    let token_record = AccessToken {
        token: access_token.clone(),
        client_id: auth_code.client_id,
        scope: auth_code.scope.clone(),
        expires_at: epoch_now() + expires_in,
    };

    if let Err(e) = store.save_token(token_record).await {
        warn!(error = %e, "failed to persist access token");
        return Err(StatusCode::INTERNAL_SERVER_ERROR);
    }

    info!("issued access token");

    Ok(Json(TokenResponse {
        access_token,
        token_type: "Bearer".into(),
        expires_in,
        scope: auth_code.scope,
    }))
}
