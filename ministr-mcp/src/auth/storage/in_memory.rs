//! Process-local, non-persistent OAuth storage backend.
//!
//! The default. Suitable for tests, single-replica development, and any
//! deployment that doesn't need OAuth state to survive restarts.

use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::RwLock;

use super::super::types::{AccessToken, AuthorizationCode, RegisteredClient};
use super::{OAuthStorage, StorageResult};

/// In-memory `OAuthStorage` implementation. Cheap to `Clone` — shared state
/// lives behind `Arc<RwLock<HashMap>>`.
#[derive(Debug, Clone, Default)]
pub(crate) struct InMemoryStorage {
    clients: Arc<RwLock<HashMap<String, RegisteredClient>>>,
    codes: Arc<RwLock<HashMap<String, AuthorizationCode>>>,
    tokens: Arc<RwLock<HashMap<String, AccessToken>>>,
}

impl InMemoryStorage {
    pub(crate) fn new() -> Self {
        Self::default()
    }
}

impl OAuthStorage for InMemoryStorage {
    async fn save_client(&self, client: RegisteredClient) -> StorageResult<()> {
        self.clients
            .write()
            .await
            .insert(client.client_id.clone(), client);
        Ok(())
    }

    async fn get_client(&self, client_id: &str) -> StorageResult<Option<RegisteredClient>> {
        Ok(self.clients.read().await.get(client_id).cloned())
    }

    async fn save_code(&self, code: AuthorizationCode) -> StorageResult<()> {
        self.codes.write().await.insert(code.code.clone(), code);
        Ok(())
    }

    async fn take_code(&self, code: &str) -> StorageResult<Option<AuthorizationCode>> {
        Ok(self.codes.write().await.remove(code))
    }

    async fn save_token(&self, token: AccessToken) -> StorageResult<()> {
        self.tokens
            .write()
            .await
            .insert(token.token.clone(), token);
        Ok(())
    }

    async fn get_token(&self, token: &str) -> StorageResult<Option<AccessToken>> {
        Ok(self.tokens.read().await.get(token).cloned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::util::epoch_now;

    fn sample_token(name: &str, scope: &str, ttl_secs: u64, expired: bool) -> AccessToken {
        let expires_at = if expired {
            epoch_now().saturating_sub(ttl_secs)
        } else {
            epoch_now() + ttl_secs
        };
        AccessToken {
            token: name.to_string(),
            client_id: "client-1".into(),
            scope: scope.into(),
            expires_at,
        }
    }

    #[tokio::test]
    async fn save_and_get_token() {
        let storage = InMemoryStorage::new();
        let token = sample_token("tok-1", "ministr:read", 3600, false);
        storage.save_token(token.clone()).await.unwrap();
        let got = storage.get_token("tok-1").await.unwrap();
        assert!(got.is_some());
        assert_eq!(got.unwrap().client_id, "client-1");
    }

    #[tokio::test]
    async fn take_code_is_idempotent_after_first_take() {
        let storage = InMemoryStorage::new();
        let code = AuthorizationCode {
            code: "abc".into(),
            client_id: "c1".into(),
            redirect_uri: "http://x".into(),
            scope: String::new(),
            code_challenge: "ch".into(),
            code_challenge_method: "S256".into(),
            expires_at: epoch_now() + 60,
        };
        storage.save_code(code).await.unwrap();
        let first = storage.take_code("abc").await.unwrap();
        let second = storage.take_code("abc").await.unwrap();
        assert!(first.is_some());
        assert!(second.is_none());
    }
}
