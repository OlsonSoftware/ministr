//! High-level façade handlers code against.
//!
//! `OAuthStore` bundles configuration with a backend and exposes the small
//! set of operations the OAuth handlers actually need. Handlers depend on
//! `OAuthStore`, not on the storage trait directly — that keeps generic
//! plumbing out of axum and centralises backend-selection logic.

use tracing::warn;

use super::OAuthConfig;
use super::storage::{InMemoryStorage, OAuthBackend, StorageResult};
use super::types::{AccessToken, AuthorizationCode, RegisteredClient};
use super::util::epoch_now;

/// Configured OAuth state plus the chosen storage backend.
///
/// `Clone` is cheap: the backend variants hold either `Arc`-wrapped state
/// (`InMemory`) or `Arc`-wrapped clients (future Cosmos backend). This makes
/// it safe to use directly as axum `State<OAuthStore>`.
#[derive(Debug, Clone)]
pub struct OAuthStore {
    config: OAuthConfig,
    backend: OAuthBackend,
}

impl OAuthStore {
    /// Construct a store backed by the in-memory backend (default).
    #[must_use]
    pub fn new(config: OAuthConfig) -> Self {
        Self {
            config,
            backend: OAuthBackend::InMemory(InMemoryStorage::new()),
        }
    }

    /// Inject a specific backend (used by `cmd_serve_http` when a persistent
    /// backend is configured).
    #[must_use]
    #[allow(dead_code)] // wired up in PR1.4 (OAuthConfig backend selection)
    pub(crate) fn with_backend(config: OAuthConfig, backend: OAuthBackend) -> Self {
        Self { config, backend }
    }

    /// Read-only view of the configuration.
    #[must_use]
    pub(crate) fn config(&self) -> &OAuthConfig {
        &self.config
    }

    // ── Client lifecycle ───────────────────────────────────────────────────

    pub(crate) async fn save_client(&self, client: RegisteredClient) -> StorageResult<()> {
        self.backend.save_client(client).await
    }

    pub(crate) async fn get_client(
        &self,
        client_id: &str,
    ) -> StorageResult<Option<RegisteredClient>> {
        self.backend.get_client(client_id).await
    }

    // ── Authorization codes ────────────────────────────────────────────────

    pub(crate) async fn save_code(&self, code: AuthorizationCode) -> StorageResult<()> {
        self.backend.save_code(code).await
    }

    pub(crate) async fn take_code(
        &self,
        code: &str,
    ) -> StorageResult<Option<AuthorizationCode>> {
        self.backend.take_code(code).await
    }

    // ── Tokens ─────────────────────────────────────────────────────────────

    pub(crate) async fn save_token(&self, token: AccessToken) -> StorageResult<()> {
        self.backend.save_token(token).await
    }

    /// Validate a bearer token. Returns the `client_id` if the token exists
    /// and has not expired.
    ///
    /// Storage backend failures are logged and treated as invalid — we
    /// degrade closed: a transient Cosmos blip rejects the request rather
    /// than letting an unauthenticated caller through.
    pub(crate) async fn validate_token(&self, token: &str) -> Option<String> {
        match self.backend.get_token(token).await {
            Ok(Some(access)) if epoch_now() <= access.expires_at => Some(access.client_id),
            Ok(_) => None,
            Err(e) => {
                warn!(error = %e, "oauth storage error during token validation; rejecting");
                None
            }
        }
    }

    /// Validate a bearer token **and** require that its scope claim contains
    /// `required_scope` as a whitespace-separated entry.
    pub(crate) async fn validate_token_with_scope(
        &self,
        token: &str,
        required_scope: &str,
    ) -> Option<String> {
        match self.backend.get_token(token).await {
            Ok(Some(access)) if epoch_now() <= access.expires_at => {
                if access.scope.split_whitespace().any(|s| s == required_scope) {
                    Some(access.client_id)
                } else {
                    None
                }
            }
            Ok(_) => None,
            Err(e) => {
                warn!(error = %e, "oauth storage error during scoped token validation; rejecting");
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn store() -> OAuthStore {
        OAuthStore::new(OAuthConfig::default())
    }

    fn token(name: &str, scope: &str, ttl_secs: u64, expired: bool) -> AccessToken {
        let expires_at = if expired {
            epoch_now().saturating_sub(ttl_secs)
        } else {
            epoch_now() + ttl_secs
        };
        AccessToken {
            token: name.into(),
            client_id: "client-1".into(),
            scope: scope.into(),
            expires_at,
        }
    }

    #[tokio::test]
    async fn validates_fresh_token() {
        let store = store();
        store
            .save_token(token("t1", "ministr:read", 3600, false))
            .await
            .unwrap();
        assert_eq!(store.validate_token("t1").await, Some("client-1".into()));
    }

    #[tokio::test]
    async fn rejects_expired_token() {
        let store = store();
        store
            .save_token(token("t1", "ministr:read", 100, true))
            .await
            .unwrap();
        assert_eq!(store.validate_token("t1").await, None);
    }

    #[tokio::test]
    async fn rejects_unknown_token() {
        assert_eq!(store().validate_token("never-issued").await, None);
    }

    #[tokio::test]
    async fn scope_matching_succeeds_for_both_listed_scopes() {
        let store = store();
        store
            .save_token(token("t1", "ministr:read ministr:bundle:read", 3600, false))
            .await
            .unwrap();
        assert_eq!(
            store
                .validate_token_with_scope("t1", "ministr:bundle:read")
                .await,
            Some("client-1".into())
        );
        assert_eq!(
            store.validate_token_with_scope("t1", "ministr:read").await,
            Some("client-1".into())
        );
    }

    #[tokio::test]
    async fn scope_missing_returns_none() {
        let store = store();
        store
            .save_token(token("t1", "ministr:read", 3600, false))
            .await
            .unwrap();
        assert_eq!(
            store
                .validate_token_with_scope("t1", "ministr:bundle:read")
                .await,
            None
        );
    }

    #[tokio::test]
    async fn scope_present_but_expired_returns_none() {
        let store = store();
        store
            .save_token(token("t1", "ministr:bundle:read", 100, true))
            .await
            .unwrap();
        assert_eq!(
            store
                .validate_token_with_scope("t1", "ministr:bundle:read")
                .await,
            None
        );
    }

    #[tokio::test]
    async fn config_round_trips() {
        let config = OAuthConfig {
            issuer: "https://test.example".into(),
            scopes_supported: vec!["ministr:read".into()],
            token_ttl: Duration::from_secs(60),
            code_ttl: Duration::from_secs(30),
        };
        let store = OAuthStore::new(config.clone());
        assert_eq!(store.config().issuer, config.issuer);
    }
}
