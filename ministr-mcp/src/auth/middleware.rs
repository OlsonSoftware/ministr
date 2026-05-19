//! Bearer-token validation middleware.
//!
//! Two layers:
//! 1. `validate_token_middleware` — requires *any* valid token.
//! 2. `validate_scope_middleware` — requires a valid token **and** a specific
//!    scope claim. Distinct response codes (401 missing token, 403 missing
//!    scope) so clients can distinguish login vs. permission failures.
//!
//! Split from handlers so SRP is preserved: this file only knows about token
//! presentation and response shape; the handler file only knows about flow
//! logic.

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::{Router, extract::Request};
use tracing::debug;

use super::OAuthConfig;
use super::store::OAuthStore;

/// Validates a bearer token. Returns 401 with an RFC 6750 `WWW-Authenticate`
/// header on failure so MCP clients know where to find the metadata.
pub async fn validate_token_middleware(
    State(store): State<OAuthStore>,
    headers: HeaderMap,
    request: Request,
    next: Next,
) -> Response {
    let Some(token) = extract_bearer(&headers) else {
        debug!("missing or malformed Authorization header");
        return unauthorized_response(store.config());
    };

    if let Some(client_id) = store.validate_token(token).await {
        debug!(client_id = %client_id, "token validated");
        next.run(request).await
    } else {
        debug!("invalid or expired token");
        unauthorized_response(store.config())
    }
}

/// State for the scope-checking middleware.
#[derive(Clone)]
pub struct ScopedState {
    pub store: OAuthStore,
    pub required_scope: String,
}

/// Validates a bearer token and enforces a specific scope. 401 if missing
/// or expired; 403 if present but lacking the required scope.
pub async fn validate_scope_middleware(
    State(state): State<ScopedState>,
    headers: HeaderMap,
    request: Request,
    next: Next,
) -> Response {
    let Some(token) = extract_bearer(&headers) else {
        debug!("missing or malformed Authorization header");
        return unauthorized_response(state.store.config());
    };

    if state.store.validate_token(token).await.is_none() {
        debug!("invalid or expired token");
        return unauthorized_response(state.store.config());
    }

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

fn extract_bearer(headers: &HeaderMap) -> Option<&str> {
    let raw = headers.get(header::AUTHORIZATION)?.to_str().ok()?;
    raw.strip_prefix("Bearer ")
}

pub(super) fn unauthorized_response(config: &OAuthConfig) -> Response {
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
