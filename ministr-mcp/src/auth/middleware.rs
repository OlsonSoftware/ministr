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
use ministr_api::TenantId;
use tracing::debug;

use super::OAuthConfig;
use super::store::OAuthStore;

/// Validates a bearer token. Returns 401 with an RFC 6750 `WWW-Authenticate`
/// header on failure so MCP clients know where to find the metadata.
///
/// On success, a [`super::tenant::Tenant`] is attached to the request
/// extensions so downstream handlers can scope queries by `subject`,
/// `org_id`, and `plan` without re-running the token lookup themselves.
pub async fn validate_token_middleware(
    State(store): State<OAuthStore>,
    headers: HeaderMap,
    mut request: Request,
    next: Next,
) -> Response {
    let Some(token) = extract_bearer(&headers) else {
        debug!("missing or malformed Authorization header");
        return unauthorized_response(store.config());
    };

    if let Some(tenant) = store.resolve_tenant(token).await {
        debug!(subject = %tenant.subject, plan = ?tenant.plan, "tenant resolved");
        // TenantId is a separate, dep-light newtype in `ministr-api`
        // so `ministr-daemon`'s activity middleware can read tenant
        // identity without depending on `ministr-mcp::auth::Tenant`.
        request
            .extensions_mut()
            .insert(TenantId(tenant.subject.clone()));
        request.extensions_mut().insert(tenant);
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
///
/// On success, a [`super::tenant::Tenant`] is attached to the request
/// extensions; same shape and rationale as [`validate_token_middleware`].
pub async fn validate_scope_middleware(
    State(state): State<ScopedState>,
    headers: HeaderMap,
    mut request: Request,
    next: Next,
) -> Response {
    let Some(token) = extract_bearer(&headers) else {
        debug!("missing or malformed Authorization header");
        return unauthorized_response(state.store.config());
    };

    // F3.4a — resolve_tenant_with_scope walks BOTH paths: OAuth token
    // first, then the API-key resolver (mst_pk_…) on miss. The earlier
    // shape pre-checked `validate_token` (OAuth-only) and 401'd before
    // ever reaching the api-key fall-through — so a valid API key was
    // rejected as "invalid or expired token". Surfaced by F-Test-1's
    // harness. We distinguish three outcomes by calling the unified
    // resolver and inspecting the failure mode:
    //   - resolve_tenant: scope-agnostic check first to disambiguate
    //     "token unknown" (401) from "token known but lacks scope" (403).
    if state.store.resolve_tenant(token).await.is_none() {
        debug!("invalid or expired token");
        return unauthorized_response(state.store.config());
    }

    if let Some(tenant) = state
        .store
        .resolve_tenant_with_scope(token, &state.required_scope)
        .await
    {
        debug!(
            subject = %tenant.subject,
            plan = ?tenant.plan,
            scope = %state.required_scope,
            "scoped tenant resolved"
        );
        request
            .extensions_mut()
            .insert(TenantId(tenant.subject.clone()));
        request.extensions_mut().insert(tenant);
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
