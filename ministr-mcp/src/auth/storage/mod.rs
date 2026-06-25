//! Storage abstraction for OAuth clients, codes, and tokens.
//!
//! # Why this exists (DIP)
//!
//! Handlers depend on the [`OAuthStorage`] trait, not on a concrete backend.
//! This lets the in-memory backend (single-process, ephemeral) and the
//! `SQLite` backend (single-replica, persistent over the Azure Files mount)
//! plug in interchangeably. A future Cosmos backend slots in the same way.
//!
//! # Why a concrete enum on top (axum ergonomics)
//!
//! axum's `State<T>` requires a single concrete type. Making the handlers
//! generic over `S: OAuthStorage` would force generic plumbing through
//! every route. Instead, [`OAuthBackend`] is a small enum that dispatches
//! to the variant. Adding a backend = adding a variant; handlers never
//! change (OCP).
//!
//! Project convention: trait methods return `impl Future + Send` instead
//! of `async fn` so static dispatch monomorphises cleanly. See
//! `QueryBackend` in `backend/mod.rs` and `Storage` in
//! `ministr-core/src/storage/traits.rs`.

mod in_memory;
pub(crate) mod postgres;
mod sqlite;

use thiserror::Error;

use super::types::{AccessToken, AuthorizationCode, RegisteredClient};

pub(super) use in_memory::InMemoryStorage;
pub(crate) use postgres::PostgresStorage;
pub(super) use sqlite::SqliteStorage;

/// Result alias for storage operations.
pub(crate) type StorageResult<T> = Result<T, StorageError>;

/// Errors that an [`OAuthStorage`] backend can surface.
#[derive(Debug, Error)]
#[allow(dead_code)] // variants surface once a persistent backend is selected
pub(crate) enum StorageError {
    /// Backend-level failure (network, I/O, internal error).
    #[error("oauth storage backend error: {0}")]
    Backend(String),
    /// Failed to (de)serialise a record at the persistence boundary.
    #[error("oauth storage serde error: {0}")]
    Serde(#[from] serde_json::Error),
}

/// The minimal contract every OAuth backend implements.
///
/// Semantics (Liskov):
/// - `save_*` is upsert-by-primary-key, idempotent.
/// - `get_*` returns the latest record or `None`.
/// - `take_code` is **atomic** get-and-delete; required for OAuth one-shot
///   code exchange. Backends must use a transaction or equivalent
///   primitive to honour this.
pub(crate) trait OAuthStorage: Send + Sync {
    fn save_client(
        &self,
        client: RegisteredClient,
    ) -> impl Future<Output = StorageResult<()>> + Send;

    fn get_client(
        &self,
        client_id: &str,
    ) -> impl Future<Output = StorageResult<Option<RegisteredClient>>> + Send;

    fn save_code(&self, code: AuthorizationCode) -> impl Future<Output = StorageResult<()>> + Send;

    /// Atomically retrieve and remove an authorization code.
    fn take_code(
        &self,
        code: &str,
    ) -> impl Future<Output = StorageResult<Option<AuthorizationCode>>> + Send;

    fn save_token(&self, token: AccessToken) -> impl Future<Output = StorageResult<()>> + Send;

    fn get_token(
        &self,
        token: &str,
    ) -> impl Future<Output = StorageResult<Option<AccessToken>>> + Send;
}

// ── Backend enum (concrete dispatch for axum State) ────────────────────────

/// Concrete backend dispatcher. Add a variant to support a new storage type.
#[derive(Debug, Clone)]
#[allow(dead_code)] // Postgres variant constructed by `cmd_serve_http`
pub(crate) enum OAuthBackend {
    InMemory(InMemoryStorage),
    Sqlite(SqliteStorage),
    Postgres(PostgresStorage),
}

impl OAuthBackend {
    pub(crate) async fn save_client(&self, client: RegisteredClient) -> StorageResult<()> {
        match self {
            Self::InMemory(s) => s.save_client(client).await,
            Self::Sqlite(s) => s.save_client(client).await,
            Self::Postgres(s) => s.save_client(client).await,
        }
    }

    pub(crate) async fn get_client(
        &self,
        client_id: &str,
    ) -> StorageResult<Option<RegisteredClient>> {
        match self {
            Self::InMemory(s) => s.get_client(client_id).await,
            Self::Sqlite(s) => s.get_client(client_id).await,
            Self::Postgres(s) => s.get_client(client_id).await,
        }
    }

    pub(crate) async fn save_code(&self, code: AuthorizationCode) -> StorageResult<()> {
        match self {
            Self::InMemory(s) => s.save_code(code).await,
            Self::Sqlite(s) => s.save_code(code).await,
            Self::Postgres(s) => s.save_code(code).await,
        }
    }

    pub(crate) async fn take_code(&self, code: &str) -> StorageResult<Option<AuthorizationCode>> {
        match self {
            Self::InMemory(s) => s.take_code(code).await,
            Self::Sqlite(s) => s.take_code(code).await,
            Self::Postgres(s) => s.take_code(code).await,
        }
    }

    pub(crate) async fn save_token(&self, token: AccessToken) -> StorageResult<()> {
        match self {
            Self::InMemory(s) => s.save_token(token).await,
            Self::Sqlite(s) => s.save_token(token).await,
            Self::Postgres(s) => s.save_token(token).await,
        }
    }

    pub(crate) async fn get_token(&self, token: &str) -> StorageResult<Option<AccessToken>> {
        match self {
            Self::InMemory(s) => s.get_token(token).await,
            Self::Sqlite(s) => s.get_token(token).await,
            Self::Postgres(s) => s.get_token(token).await,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn backend_enum_dispatches() {
        use super::super::util::epoch_now;
        let backend = OAuthBackend::InMemory(InMemoryStorage::new());
        let token = AccessToken {
            token: "via-enum".into(),
            client_id: "client-1".into(),
            scope: "ministr:read".into(),
            expires_at: epoch_now() + 3600,
        };
        backend.save_token(token).await.unwrap();
        assert!(backend.get_token("via-enum").await.unwrap().is_some());
    }
}
