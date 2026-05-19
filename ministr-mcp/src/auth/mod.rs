//! OAuth 2.1 authorization framework for remote Streamable HTTP deployment.
//!
//! Implements the MCP authorization specification (2025-06-18) with:
//! - OAuth 2.0 Protected Resource Metadata (RFC 9728)
//! - Authorization Code flow with PKCE (S256)
//! - Dynamic Client Registration (RFC 7591)
//! - Bearer token validation middleware
//!
//! # Architecture (SOLID layering)
//!
//! ```text
//!     router.rs    — composes axum Routers
//!         │
//!     handlers.rs  — per-endpoint flow logic, uses OAuthStore façade
//!     middleware.rs — bearer token + scope checks, uses OAuthStore façade
//!         │
//!     store.rs     — OAuthStore: thin façade exposing only what handlers need
//!         │
//!     storage.rs   — OAuthStorage trait + OAuthBackend enum + InMemoryStorage
//!         │
//!     types.rs     — RegisteredClient, AuthorizationCode, AccessToken
//! ```
//!
//! Backends plug in by adding a variant to `OAuthBackend` and an impl of
//! `OAuthStorage`. Handlers never change (OCP).

use std::time::Duration;

pub mod middleware;
pub mod router;
pub mod tenant;

mod handlers;
mod storage;
mod store;
mod types;
mod util;

pub use middleware::{
    ScopedState, scope_protected_router, validate_scope_middleware, validate_token_middleware,
};
pub use router::{oauth_routes, protected_router};
pub use store::OAuthStore;
pub use tenant::{Plan, Tenant};

/// OAuth 2.1 server configuration.
#[derive(Debug, Clone)]
pub struct OAuthConfig {
    /// The issuer URL (e.g., `https://ministr.example.com`).
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
                "ministr:read".into(),
                "ministr:write".into(),
                "ministr:bundle:read".into(),
                "ministr:bundle:write".into(),
            ],
            token_ttl: Duration::from_secs(3600),
            code_ttl: Duration::from_secs(600),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_has_required_scopes() {
        let config = OAuthConfig::default();
        for required in [
            "ministr:read",
            "ministr:write",
            "ministr:bundle:read",
            "ministr:bundle:write",
        ] {
            assert!(
                config.scopes_supported.iter().any(|s| s == required),
                "missing scope {required}"
            );
        }
        assert_eq!(config.token_ttl, Duration::from_secs(3600));
    }
}
