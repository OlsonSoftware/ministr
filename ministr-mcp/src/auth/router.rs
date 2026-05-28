//! Router builders.
//!
//! Two compositions exposed:
//! - `oauth_routes` — the public discovery + flow endpoints.
//! - `protected_router` — public routes merged with a token-protected
//!   MCP router.

use axum::Router;
use axum::middleware;
use axum::routing::{get, post};

use super::handlers::{
    authorization_server_metadata, authorize, protected_resource_metadata, register_client, token,
};
use super::middleware::validate_token_middleware;
use super::store::OAuthStore;

/// Build the public OAuth 2.1 routes (discovery, registration, authorize, token).
pub fn oauth_routes(store: OAuthStore) -> Router {
    Router::new()
        .route(
            "/.well-known/oauth-protected-resource",
            get(protected_resource_metadata),
        )
        .route(
            "/.well-known/oauth-authorization-server",
            get(authorization_server_metadata),
        )
        .route("/oauth/register", post(register_client))
        .route("/oauth/authorize", get(authorize))
        .route("/oauth/token", post(token))
        .with_state(store)
}

/// Build a complete OAuth-protected router: merges public OAuth endpoints
/// with a protected MCP service that requires Bearer token authentication.
pub fn protected_router(mcp_router: Router, store: OAuthStore) -> Router {
    let public = oauth_routes(store.clone());

    let protected = mcp_router.layer(middleware::from_fn_with_state(
        store,
        validate_token_middleware,
    ));

    public.merge(protected)
}
